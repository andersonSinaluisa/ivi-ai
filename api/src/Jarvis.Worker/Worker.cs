using System.Diagnostics;
using System.Security.Cryptography;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Jobs;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Worker;

/// <summary>Configuración del worker (sección Jarvis:Worker).</summary>
public sealed class WorkerOptions
{
    public const string Section = "Jarvis:Worker";

    public int PollSeconds { get; set; } = 10;

    public int MaxConcurrentRuns { get; set; } = 2;

    /// <summary>Directorio de backups montado en solo lectura (lo escribe deploy/scripts/backup.sh en el host).</summary>
    public string BackupDirectory { get; set; } = "/backups";

    public string HeartbeatPath { get; set; } = "/tmp/jarvis-worker.alive";

    /// <summary>Allowlist de http.check: nombre → URL. Los jobs solo pueden nombrar una entrada, nunca pasar una URL.</summary>
    public Dictionary<string, string> HttpChecks { get; set; } = new(StringComparer.Ordinal);
}

/// <summary>Bucle del worker (ADR-016): programa, reclama y ejecuta; escribe un latido para el healthcheck.</summary>
public sealed class JobWorker(IServiceScopeFactory scopes, IOptions<WorkerOptions> options, ILogger<JobWorker> logger) : BackgroundService
{
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var settings = options.Value;
        using var slots = new SemaphoreSlim(Math.Max(1, settings.MaxConcurrentRuns));
        var running = new List<Task>();
        logger.LogInformation("Jarvis worker started (poll {Poll}s, {Slots} concurrent runs)", settings.PollSeconds, settings.MaxConcurrentRuns);
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                await using (var scope = scopes.CreateAsyncScope())
                {
                    await scope.ServiceProvider.GetRequiredService<JobScheduler>().ScheduleDueAsync(stoppingToken);
                }

                IReadOnlyList<Guid> claimed;
                await using (var scope = scopes.CreateAsyncScope())
                {
                    claimed = await scope.ServiceProvider.GetRequiredService<JobScheduler>().ClaimAsync(stoppingToken);
                }

                foreach (var runId in claimed)
                {
                    await slots.WaitAsync(stoppingToken);
                    running.Add(Task.Run(
                        async () =>
                        {
                            try
                            {
                                await using var scope = scopes.CreateAsyncScope();
                                await scope.ServiceProvider.GetRequiredService<JobScheduler>().ExecuteAsync(runId, stoppingToken);
                            }
                            catch (Exception exception) when (exception is not OperationCanceledException)
                            {
                                // La lease vencerá y otro ciclo lo reintentará.
                                logger.LogError(exception, "Run {RunId} crashed", runId);
                            }
                            finally
                            {
                                slots.Release();
                            }
                        },
                        stoppingToken));
                }

                running.RemoveAll(t => t.IsCompleted);
                await File.WriteAllTextAsync(settings.HeartbeatPath, DateTimeOffset.UtcNow.ToString("O"), stoppingToken);
            }
            catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
            {
                break;
            }
            catch (Exception exception)
            {
                logger.LogError(exception, "Worker cycle failed; retrying");
            }

            try
            {
                await Task.Delay(TimeSpan.FromSeconds(settings.PollSeconds), stoppingToken);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }

        await Task.WhenAll(running);
    }
}

/// <summary>
/// backup.verify — solo lectura. Comprueba en el directorio de backups (montado read-only): el último éxito
/// (<c>.last-success</c>) es reciente, el último dump cifrado existe, tiene cabecera de age y su SHA-256 coincide con
/// <c>SHA256SUMS</c>. La restauración de prueba completa se hace fuera del VPS (allí no está la clave privada).
/// </summary>
public sealed class BackupVerifyJob(IOptions<WorkerOptions> options, TimeProvider clock) : IJobHandler
{
    public string Type => JobCatalog.BackupVerify;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        var directory = options.Value.BackupDirectory;
        var maxAgeHours = context.Parameters.TryGetProperty("maxAgeHours", out var p) && p.TryGetInt32(out var h) ? h : 26;
        var problems = new List<string>();
        if (!Directory.Exists(directory))
        {
            throw new InvalidOperationException($"Backup directory '{directory}' is not mounted.");
        }

        var marker = Path.Combine(directory, ".last-success");
        var lastSuccess = File.Exists(marker) ? File.GetLastWriteTimeUtc(marker) : (DateTime?)null;
        var ageHours = lastSuccess is null ? (double?)null : (clock.GetUtcNow().UtcDateTime - lastSuccess.Value).TotalHours;
        if (ageHours is null || ageHours > maxAgeHours)
        {
            problems.Add(ageHours is null ? "no successful backup recorded" : $"last backup is {ageHours:F1} h old (limit {maxAgeHours} h)");
        }

        var latest = new DirectoryInfo(directory).GetFiles("jarvis-*.dump.age").OrderByDescending(f => f.Name, StringComparer.Ordinal).FirstOrDefault();
        string? sha = null;
        if (latest is null)
        {
            problems.Add("no encrypted database dump found");
        }
        else
        {
            if (latest.Length < 100)
            {
                problems.Add($"{latest.Name} is too small ({latest.Length} bytes)");
            }

            await using var stream = latest.OpenRead();
            var header = new byte[21];
            var read = await stream.ReadAsync(header, cancellationToken);
            if (read < header.Length || System.Text.Encoding.ASCII.GetString(header) != "age-encryption.org/v1")
            {
                problems.Add($"{latest.Name} does not have an age header");
            }

            stream.Position = 0;
            sha = Convert.ToHexStringLower(await SHA256.HashDataAsync(stream, cancellationToken));
            var sums = Path.Combine(directory, "SHA256SUMS");
            if (!File.Exists(sums))
            {
                problems.Add("SHA256SUMS is missing (backup.sh writes it)");
            }
            else
            {
                var expected = (await File.ReadAllLinesAsync(sums, cancellationToken))
                    .Select(l => l.Split(' ', 2, StringSplitOptions.RemoveEmptyEntries))
                    .FirstOrDefault(parts => parts.Length == 2 && parts[1].TrimStart('*').Trim() == latest.Name)?[0];
                if (expected is null)
                {
                    problems.Add($"{latest.Name} is not listed in SHA256SUMS");
                }
                else if (!string.Equals(expected, sha, StringComparison.OrdinalIgnoreCase))
                {
                    problems.Add($"{latest.Name} checksum mismatch");
                }
            }
        }

        if (problems.Count > 0)
        {
            throw new InvalidOperationException("Backup verification failed: " + string.Join("; ", problems));
        }

        return new JobOutcome(
            new { ok = true, latest = latest!.Name, sizeBytes = latest.Length, ageHours = Math.Round(ageHours!.Value, 1) },
            new { sha256 = sha });
    }
}

/// <summary>http.check — solo lectura. GET a una URL de la allowlist de configuración (nunca una URL del job).</summary>
public sealed class HttpCheckJob(IOptions<WorkerOptions> options) : IJobHandler
{
    private static readonly HttpClient Client = new(new SocketsHttpHandler { AllowAutoRedirect = false, ConnectTimeout = TimeSpan.FromSeconds(5) })
    {
        Timeout = TimeSpan.FromSeconds(15),
    };

    public string Type => JobCatalog.HttpCheck;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        var target = context.Parameters.GetProperty("target").GetString()!;
        if (!options.Value.HttpChecks.TryGetValue(target, out var url) || !Uri.TryCreate(url, UriKind.Absolute, out var uri) || uri.Scheme is not ("https" or "http"))
        {
            throw new InvalidOperationException($"'{target}' is not in the worker's http check allowlist.");
        }

        var watch = Stopwatch.StartNew();
        using var response = await Client.GetAsync(uri, HttpCompletionOption.ResponseHeadersRead, cancellationToken);
        watch.Stop();
        var status = (int)response.StatusCode;
        if (status >= 500)
        {
            throw new InvalidOperationException($"{target} answered {status}.");
        }

        return new JobOutcome(new { target, status, latencyMs = watch.ElapsedMilliseconds, ok = status < 400 });
    }
}

public static class WorkerHost
{
    public static HostApplicationBuilder CreateBuilder(string[] args, Action<IConfigurationBuilder>? configure = null)
    {
        var builder = Host.CreateApplicationBuilder(args);
        // worker.json (y no appsettings.json) para no chocar con la configuración de la API en salidas compartidas.
        builder.Configuration.AddJsonFile(Path.Combine(AppContext.BaseDirectory, "worker.json"), optional: true);
        builder.Configuration.AddEnvironmentVariables();
        configure?.Invoke(builder.Configuration);
        Jarvis.Infrastructure.SecretFileConfiguration.AddSecretFiles(builder.Configuration);
        if (builder.Configuration.GetValue("Logging:Console:FormatterName", "json") == "json")
        {
            builder.Logging.ClearProviders();
            builder.Logging.AddJsonConsole(o =>
            {
                o.IncludeScopes = true;
                o.UseUtcTimestamp = true;
            });
        }

        var services = builder.Services;
        services.AddOptions<WorkerOptions>().Bind(builder.Configuration.GetSection(WorkerOptions.Section));
        services.AddSingleton(TimeProvider.System);
        services.AddScoped<IRequestContext, RequestContext>();
        Jarvis.Infrastructure.DependencyInjection.AddJarvisPersistence(services, builder.Configuration);
        services.AddSingleton(new JobSchedulerOptions());
        Jarvis.Application.DependencyInjection.AddJarvisJobs(services);
        services.AddScoped<IJobHandler, BackupVerifyJob>();
        services.AddScoped<IJobHandler, HttpCheckJob>();
        services.AddHostedService<JobWorker>();
        return builder;
    }

    /// <summary>Healthcheck del contenedor (la imagen no tiene shell): el latido debe tener menos de 3 ciclos.</summary>
    public static int Healthcheck(string[] args)
    {
        var configuration = new ConfigurationBuilder()
            .AddJsonFile(Path.Combine(AppContext.BaseDirectory, "worker.json"), optional: true)
            .AddEnvironmentVariables()
            .AddCommandLine(args)
            .Build();
        var settings = configuration.GetSection(WorkerOptions.Section).Get<WorkerOptions>() ?? new WorkerOptions();
        if (!File.Exists(settings.HeartbeatPath))
        {
            return 1;
        }

        var age = DateTime.UtcNow - File.GetLastWriteTimeUtc(settings.HeartbeatPath);
        return age < TimeSpan.FromSeconds(Math.Max(60, settings.PollSeconds * 3)) ? 0 : 1;
    }
}
