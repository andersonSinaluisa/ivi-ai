using Jarvis.Application.Auth;
using Jarvis.Infrastructure;
using Jarvis.Infrastructure.Mqtt;
using Jarvis.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Api.Commands;

/// <summary>Subcomandos de administración del mismo binario (<c>jarvis-api &lt;comando&gt;</c>).</summary>
public static class AdminCommands
{
    public static int Usage(string? command)
    {
        Console.Error.WriteLine($"Unknown command '{command}'.");
        Console.Error.WriteLine("Usage: jarvis-api [migrate | bootstrap-owner --email <e> --display-name <n> | broker-bootstrap ... | healthcheck]");
        return 2;
    }

    /// <summary>Aplica migraciones con <c>ConnectionStrings:PostgresMigrator</c> (rol jarvis_migrator) o, si falta, la de la app.</summary>
    public static async Task<int> MigrateAsync(string[] args)
    {
        await using var app = BuildToolHost(args, useMigrator: true);
        await using var scope = app.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
        var pending = (await db.Database.GetPendingMigrationsAsync()).ToList();
        Console.WriteLine($"Applying {pending.Count} migration(s)...");
        await db.Database.MigrateAsync();
        Console.WriteLine("Database is up to date.");
        return 0;
    }

    /// <summary>HU-008: la contraseña se lee de stdin (nunca como argumento).</summary>
    public static async Task<int> BootstrapOwnerAsync(string[] args)
    {
        var email = Option(args, "--email");
        var displayName = Option(args, "--display-name") ?? "Owner";
        if (email is null)
        {
            Console.Error.WriteLine("Usage: jarvis-api bootstrap-owner --email <email> [--display-name <name>]  (password from stdin)");
            return 2;
        }

        if (!Console.IsInputRedirected)
        {
            Console.Error.Write("Password: ");
        }

        var password = ReadSecretLine();
        await using var app = BuildToolHost(args.Where(a => !a.StartsWith("--email", StringComparison.Ordinal)).ToArray(), useMigrator: false);
        await using var scope = app.Services.CreateAsyncScope();
        var result = await scope.ServiceProvider.GetRequiredService<BootstrapOwnerHandler>().HandleAsync(email, displayName, password, CancellationToken.None);
        if (!result.IsSuccess)
        {
            var details = result.Error!.Errors is null ? string.Empty : " " + string.Join("; ", result.Error.Errors.SelectMany(e => e.Value));
            Console.Error.WriteLine($"{result.Error.Code}: {result.Error.Message}{details}");
            return 1;
        }

        Console.WriteLine($"Owner created: {result.Value}");
        return 0;
    }

    /// <summary>Bootstrap de Dynamic Security con el admin temporal (docs/05-security/secrets-management.md).</summary>
    public static async Task<int> BrokerBootstrapAsync(string[] args)
    {
        var host = Option(args, "--host") ?? "mosquitto";
        var port = int.Parse(Option(args, "--port") ?? "1883", System.Globalization.CultureInfo.InvariantCulture);
        var admin = Option(args, "--admin-user") ?? "bootstrap-admin";
        var adminPasswordFile = Option(args, "--admin-password-file");
        var cloudPasswordFile = Option(args, "--cloud-password-file");
        if (adminPasswordFile is null || cloudPasswordFile is null)
        {
            Console.Error.WriteLine("Usage: jarvis-api broker-bootstrap --admin-password-file <f> --cloud-password-file <f> [--host h] [--port p] [--admin-user u] [--delete-admin]");
            return 2;
        }

        using var loggerFactory = LoggerFactory.Create(b => b.AddSimpleConsole());
        var bootstrapper = new DynsecBootstrapper(loggerFactory.CreateLogger("broker-bootstrap"));
        await bootstrapper.RunAsync(
            host,
            port,
            admin,
            (await File.ReadAllTextAsync(adminPasswordFile)).Trim(),
            (await File.ReadAllTextAsync(cloudPasswordFile)).Trim(),
            args.Contains("--delete-admin"),
            CancellationToken.None);
        return 0;
    }

    /// <summary>Healthcheck del contenedor (la imagen chiseled no tiene curl).</summary>
    public static async Task<int> HealthcheckAsync(string[] args)
    {
        var url = Option(args, "--url") ?? "http://127.0.0.1:8080/health/ready";
        using var http = new HttpClient { Timeout = TimeSpan.FromSeconds(5) };
        try
        {
            using var response = await http.GetAsync(new Uri(url));
            return response.IsSuccessStatusCode ? 0 : 1;
        }
        catch (HttpRequestException)
        {
            return 1;
        }
        catch (TaskCanceledException)
        {
            return 1;
        }
    }

    private static WebApplication BuildToolHost(string[] args, bool useMigrator)
    {
        var builder = WebApplication.CreateBuilder(args);
        builder.Configuration.AddSecretFiles();
        if (useMigrator && builder.Configuration.GetConnectionString("PostgresMigrator") is { Length: > 0 } migrator)
        {
            builder.Configuration["ConnectionStrings:Postgres"] = migrator;
        }

        builder.Logging.SetMinimumLevel(LogLevel.Warning);
        JarvisApiHost.ConfigureServices(builder, enableBackgroundServices: false);
        return builder.Build();
    }

    private static string? Option(string[] args, string name)
    {
        var index = Array.IndexOf(args, name);
        return index >= 0 && index + 1 < args.Length ? args[index + 1] : null;
    }

    private static string ReadSecretLine()
    {
        if (Console.IsInputRedirected)
        {
            return Console.In.ReadLine() ?? string.Empty;
        }

        var buffer = new System.Text.StringBuilder();
        while (true)
        {
            var key = Console.ReadKey(intercept: true);
            if (key.Key == ConsoleKey.Enter)
            {
                Console.Error.WriteLine();
                return buffer.ToString();
            }

            if (key.Key == ConsoleKey.Backspace && buffer.Length > 0)
            {
                buffer.Length--;
            }
            else if (!char.IsControl(key.KeyChar))
            {
                buffer.Append(key.KeyChar);
            }
        }
    }
}
