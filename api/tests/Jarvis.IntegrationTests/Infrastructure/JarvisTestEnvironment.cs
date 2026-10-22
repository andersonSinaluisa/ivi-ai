using System.Collections.Concurrent;
using System.Net;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Text;
using Jarvis.Api;
using Jarvis.Infrastructure.Mqtt;
using Jarvis.Infrastructure.Persistence;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.TestHost;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace Jarvis.IntegrationTests.Infrastructure;

/// <summary>
/// PostgreSQL 17 (con pgvector) + Mosquitto 2.0 reales con la configuración de producción de deploy/, Dynamic Security
/// inicializado con el mismo bootstrap que producción, TLS con CA de prueba y la API en TestServer. Los servicios corren
/// en contenedores (Testcontainers) o, sin Docker, como procesos nativos (<see cref="TestServices"/>). Si ninguno está
/// disponible, <see cref="Available"/> es false y las pruebas se omiten.
/// </summary>
public sealed class JarvisTestEnvironment : IAsyncLifetime
{
    public const string AdminPassword = "bootstrap-admin-password-for-tests";

    private ITestServices? _services;
    private WebApplication? _app;
    private Dictionary<string, string?>? _settings;

    public bool Available { get; private set; }

    public string? UnavailableReason { get; private set; }

    public string CloudPassword { get; } = Convert.ToHexString(RandomNumberGenerator.GetBytes(16));

    public string MqttHost { get; private set; } = "127.0.0.1";

    public int MqttTlsPort { get; private set; }

    public int MqttPlainPort { get; private set; }

    /// <summary>CA de prueba en PEM (los simuladores la usan como raíz de confianza).</summary>
    public string CaFile { get; private set; } = string.Empty;

    public string StateDirectory { get; } = Path.Combine(Path.GetTempPath(), "jarvis-tests", Guid.NewGuid().ToString("N"));

    public CapturingLoggerProvider Logs { get; } = new();

    public WebApplication App => _app ?? throw new InvalidOperationException("Environment not started.");

    public IServiceProvider Services => App.Services;

    public HttpClient CreateClient() => App.GetTestClient();

    public HttpMessageHandler CreateHandler() => App.GetTestServer().CreateHandler();

    public Uri ServerBaseAddress => App.GetTestServer().BaseAddress;

    public async ValueTask InitializeAsync()
    {
        Directory.CreateDirectory(StateDirectory);
        try
        {
            await StartContainersAsync();
        }
        catch (Exception exception) when (exception is not OutOfMemoryException)
        {
            UnavailableReason = $"Test services unavailable: {exception.GetType().Name}: {exception.Message}";
            return;
        }

        await new DynsecBootstrapper(NullLogger.Instance).RunAsync(
            MqttHost, MqttPlainPort, "bootstrap-admin", AdminPassword, CloudPassword, deleteAdmin: true, CancellationToken.None);

        _settings = Settings();
        await EnableVectorExtensionAsync();
        await StartApiAsync(migrate: true);
        Available = true;
    }

    /// <summary>Como provision.sql en producción: pgvector lo crea el superusuario, no la migración.</summary>
    public bool VectorExtension { get; private set; }

    private async Task EnableVectorExtensionAsync()
    {
        await using var connection = new Npgsql.NpgsqlConnection(_services!.PostgresConnectionString);
        await connection.OpenAsync();
        await using var check = new Npgsql.NpgsqlCommand("SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector')", connection);
        if ((bool)(await check.ExecuteScalarAsync())!)
        {
            await using var create = new Npgsql.NpgsqlCommand("CREATE EXTENSION IF NOT EXISTS vector", connection);
            await create.ExecuteNonQueryAsync();
            VectorExtension = true;
        }
    }

    /// <summary>Detiene la API (el cloud client deja su sesión MQTT persistente en el broker).</summary>
    public async Task StopApiAsync()
    {
        if (_app is not null)
        {
            await _app.StopAsync();
            await _app.DisposeAsync();
            _app = null;
        }
    }

    public async Task StartApiAsync(bool migrate = false)
    {
        var builder = JarvisApiHost.CreateBuilder(["--environment", "Testing"], enableBackgroundServices: true, configure: config =>
            config.AddInMemoryCollection(_settings!));
        builder.WebHost.UseTestServer();
        builder.Logging.ClearProviders();
        builder.Logging.AddProvider(Logs);
        builder.Logging.SetMinimumLevel(LogLevel.Debug);
        builder.Logging.AddFilter("Microsoft", LogLevel.Warning);
        var app = JarvisApiHost.BuildApp(builder);
        if (migrate)
        {
            await using var scope = app.Services.CreateAsyncScope();
            await scope.ServiceProvider.GetRequiredService<JarvisDbContext>().Database.MigrateAsync();
        }

        await app.StartAsync();
        _app = app;
        await WaitUntilAsync(() => app.Services.GetRequiredService<MqttCloudConnection>().IsConnected, TimeSpan.FromSeconds(30), "cloud MQTT connection");
    }

    public async ValueTask DisposeAsync()
    {
        if (_app is not null)
        {
            await _app.StopAsync();
            await _app.DisposeAsync();
        }

        if (_services is not null)
        {
            await _services.DisposeAsync();
        }

        Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
        try
        {
            Directory.Delete(StateDirectory, recursive: true);
        }
        catch (IOException)
        {
        }
    }

    /// <summary>"containers" o "native".</summary>
    public string ServicesKind => _services?.Kind ?? "none";

    /// <summary>Cadena de conexión del superusuario de la base de pruebas.</summary>
    public string PostgresConnectionString => _services?.PostgresConnectionString ?? throw new InvalidOperationException("Environment not started.");

    public async Task RestartMosquittoAsync()
    {
        await _services!.StopBrokerAsync();
        await _services.StartBrokerAsync();
    }

    public Task StopMosquittoAsync() => _services!.StopBrokerAsync();

    public Task StartMosquittoAsync() => _services!.StartBrokerAsync();

    public static async Task WaitUntilAsync(Func<bool> condition, TimeSpan timeout, string? because = null)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (!condition())
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException($"Condition not met within {timeout}{(because is null ? string.Empty : ": " + because)}.");
            }

            await Task.Delay(100);
        }
    }

    public static async Task<T> WaitForAsync<T>(Func<Task<T?>> probe, TimeSpan timeout, string because)
        where T : class
    {
        var deadline = DateTime.UtcNow + timeout;
        while (true)
        {
            var value = await probe();
            if (value is not null)
            {
                return value;
            }

            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException($"Timed out waiting for: {because}");
            }

            await Task.Delay(150);
        }
    }

    private Dictionary<string, string?> Settings() => new()
    {
        ["Logging:Console:FormatterName"] = "simple",
        ["ConnectionStrings:Postgres"] = _services!.PostgresConnectionString,
        ["Jwt:SigningKey"] = Convert.ToBase64String(RandomNumberGenerator.GetBytes(48)),
        ["Jarvis:DeviceSecretPepper"] = Convert.ToBase64String(RandomNumberGenerator.GetBytes(48)),
        ["Jarvis:Mqtt:Host"] = MqttHost,
        ["Jarvis:Mqtt:Port"] = MqttPlainPort.ToString(System.Globalization.CultureInfo.InvariantCulture),
        ["Jarvis:Mqtt:Password"] = CloudPassword,
        ["Jarvis:Mqtt:PublicHost"] = MqttHost,
        ["Jarvis:Mqtt:PublicPort"] = MqttTlsPort.ToString(System.Globalization.CultureInfo.InvariantCulture),
        ["Jarvis:Mqtt:InstanceId"] = "tests",
        ["Jarvis:Tasks:AckTimeoutSeconds"] = "3",
        ["Jarvis:Tasks:DispatchIntervalSeconds"] = "1",
        ["Jarvis:Tasks:DeliveryTimeoutSweepSeconds"] = "1",
        ["Jarvis:Tasks:ExpirySweepSeconds"] = "1",
        ["Jarvis:Tasks:CancelResendSeconds"] = "5",
        ["Jarvis:Tasks:CreateLimitPerMinute"] = "1000",
        ["Jarvis:Presence:HeartbeatIntervalSeconds"] = "1",
        ["Jarvis:Presence:StaleAfterSeconds"] = "4",
        ["Jarvis:Presence:OfflineAfterSeconds"] = "8",
        ["Jarvis:Presence:SweepIntervalSeconds"] = "1",
        ["Jarvis:Security:PrincipalStatusCacheSeconds"] = "2",
        ["Jarvis:Security:MqttMessagesPerSecond"] = "200",
        ["Jarvis:Security:AuthRequestsPerMinute"] = "100000",
        ["Jarvis:BrokerReconciliation:RetrySeconds"] = "2",
        ["Jarvis:Retention:Enabled"] = "false",
    };

    private async Task StartContainersAsync()
    {
        var (caPem, certPem, keyPem) = CreateTlsMaterial();
        CaFile = Path.Combine(StateDirectory, "test-ca.pem");
        await File.WriteAllTextAsync(CaFile, caPem);

        _services = await TestServices.StartAsync(StateDirectory, AdminPassword, new TlsMaterial(caPem, certPem, keyPem));
        MqttHost = "127.0.0.1";
        MqttPlainPort = _services.MqttPlainPort;
        MqttTlsPort = _services.MqttTlsPort;
    }

    private static (string Ca, string Cert, string Key) CreateTlsMaterial()
    {
        using var caKey = RSA.Create(2048);
        var caRequest = new CertificateRequest("CN=Jarvis Test CA", caKey, HashAlgorithmName.SHA256, RSASignaturePadding.Pkcs1);
        caRequest.CertificateExtensions.Add(new X509BasicConstraintsExtension(true, false, 0, true));
        caRequest.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.KeyCertSign | X509KeyUsageFlags.CrlSign, true));
        // Un solo instante de referencia: el certificado del servidor no puede vencer después que su CA.
        var now = DateTimeOffset.UtcNow;
        using var ca = caRequest.CreateSelfSigned(now.AddDays(-1), now.AddDays(31));

        using var serverKey = RSA.Create(2048);
        var request = new CertificateRequest("CN=localhost", serverKey, HashAlgorithmName.SHA256, RSASignaturePadding.Pkcs1);
        var san = new SubjectAlternativeNameBuilder();
        san.AddDnsName("localhost");
        san.AddIpAddress(IPAddress.Loopback);
        request.CertificateExtensions.Add(san.Build());
        request.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension([new Oid("1.3.6.1.5.5.7.3.1")], false));
        using var server = request.Create(ca, now.AddDays(-1), now.AddDays(30), RandomNumberGenerator.GetBytes(16));

        return (ca.ExportCertificatePem(), server.ExportCertificatePem() + "\n" + ca.ExportCertificatePem(), serverKey.ExportPkcs8PrivateKeyPem());
    }

    public static string RepoRoot()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null && !File.Exists(Path.Combine(directory.FullName, "Jarvis.slnx")))
        {
            directory = directory.Parent;
        }

        return directory?.FullName ?? throw new InvalidOperationException("Repository root not found.");
    }
}

/// <summary>Captura toda la salida de logs (incluidos scopes) para verificar redacción de secretos.</summary>
public sealed class CapturingLoggerProvider : ILoggerProvider, ISupportExternalScope
{
    private readonly ConcurrentQueue<string> _lines = new();
    private IExternalScopeProvider _scopes = new LoggerExternalScopeProvider();

    public IReadOnlyCollection<string> Lines => _lines;

    public string AllText() => string.Join('\n', _lines);

    public ILogger CreateLogger(string categoryName) => new CapturingLogger(this, categoryName);

    public void SetScopeProvider(IExternalScopeProvider scopeProvider) => _scopes = scopeProvider;

    public void Dispose()
    {
    }

    private sealed class CapturingLogger(CapturingLoggerProvider provider, string category) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => provider._scopes.Push(state);

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            var scopes = new StringBuilder();
            provider._scopes.ForEachScope((scope, sb) => sb.Append(" | ").Append(ScopeText(scope)), scopes);
            provider._lines.Enqueue($"{logLevel} {category}: {formatter(state, exception)} {exception}{scopes}");
        }

        private static string? ScopeText(object? scope) => scope is IEnumerable<KeyValuePair<string, object?>> pairs
            ? string.Join(",", pairs.Select(p => $"{p.Key}={p.Value}"))
            : scope?.ToString();
    }
}
