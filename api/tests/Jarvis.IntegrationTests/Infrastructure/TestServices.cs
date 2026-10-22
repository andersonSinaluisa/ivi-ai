using System.Diagnostics;
using System.Globalization;
using System.Net;
using System.Net.Sockets;
using System.Text;
using DotNet.Testcontainers.Builders;
using DotNet.Testcontainers.Containers;
using Testcontainers.PostgreSql;

namespace Jarvis.IntegrationTests.Infrastructure;

/// <summary>
/// PostgreSQL (con pgvector) y Mosquitto para las pruebas. Dos implementaciones con la misma configuración de
/// producción de deploy/: contenedores (Testcontainers, la opción por defecto) o procesos nativos del sistema
/// (para entornos sin Docker). Se elige con JARVIS_TEST_SERVICES=containers|native; sin la variable se intenta
/// Docker y, si no está, procesos nativos.
/// </summary>
public interface ITestServices : IAsyncDisposable
{
    string Kind { get; }

    string PostgresConnectionString { get; }

    int MqttPlainPort { get; }

    int MqttTlsPort { get; }

    Task StopBrokerAsync();

    Task StartBrokerAsync();
}

public sealed record TlsMaterial(string CaPem, string CertificatePem, string KeyPem);

public static class TestServices
{
    /// <summary>Imagen con PostgreSQL 17 y la extensión pgvector (el esquema de memoria la necesita).</summary>
    public const string PostgresImage = "pgvector/pgvector:0.8.1-pg17";

    public const string MosquittoImage = "eclipse-mosquitto:2.0.22";

    public static async Task<ITestServices> StartAsync(string stateDirectory, string adminPassword, TlsMaterial tls)
    {
        var requested = Environment.GetEnvironmentVariable("JARVIS_TEST_SERVICES")?.Trim().ToLowerInvariant();
        if (requested == "native")
        {
            return await NativeTestServices.StartAsync(stateDirectory, adminPassword, tls);
        }

        try
        {
            return await ContainerTestServices.StartAsync(adminPassword, tls);
        }
        catch (Exception containerError) when (requested != "containers" && containerError is not OutOfMemoryException)
        {
            if (!NativeTestServices.IsAvailable(out var missing))
            {
                throw new InvalidOperationException(
                    $"Docker unavailable ({containerError.GetType().Name}: {containerError.Message}) and native services unavailable ({missing}).",
                    containerError);
            }

            return await NativeTestServices.StartAsync(stateDirectory, adminPassword, tls);
        }
    }

    public static int FreePort()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var port = ((IPEndPoint)listener.LocalEndpoint).Port;
        listener.Stop();
        return port;
    }

    public static string DeployDirectory(string name) => Path.Combine(JarvisTestEnvironment.RepoRoot(), "deploy", name);
}

/// <summary>Testcontainers: postgres + mosquitto con los archivos de deploy/ montados.</summary>
internal sealed class ContainerTestServices : ITestServices
{
    private readonly PostgreSqlContainer _postgres;
    private readonly IContainer _mosquitto;

    private ContainerTestServices(PostgreSqlContainer postgres, IContainer mosquitto)
    {
        _postgres = postgres;
        _mosquitto = mosquitto;
    }

    public string Kind => "containers";

    public string PostgresConnectionString => _postgres.GetConnectionString();

    public int MqttPlainPort => _mosquitto.GetMappedPublicPort(1883);

    public int MqttTlsPort => _mosquitto.GetMappedPublicPort(8883);

    public static async Task<ContainerTestServices> StartAsync(string adminPassword, TlsMaterial tls)
    {
        var postgres = new PostgreSqlBuilder(TestServices.PostgresImage).Build();
        var deploy = TestServices.DeployDirectory("mosquitto");
        var mosquitto = new ContainerBuilder(TestServices.MosquittoImage)
            .WithEntrypoint("/bin/sh", "/mosquitto/config/docker-entrypoint.sh")
            .WithEnvironment("DYNSEC_ADMIN_USER", "bootstrap-admin")
            .WithEnvironment("DYNSEC_ADMIN_PASSWORD_FILE", "/run/secrets/admin")
            .WithResourceMapping(await File.ReadAllBytesAsync(Path.Combine(deploy, "mosquitto.conf")), "/mosquitto/config/mosquitto.conf")
            .WithResourceMapping(await File.ReadAllBytesAsync(Path.Combine(deploy, "docker-entrypoint.sh")), "/mosquitto/config/docker-entrypoint.sh")
            .WithResourceMapping(Encoding.UTF8.GetBytes(adminPassword), "/run/secrets/admin")
            .WithResourceMapping(Encoding.UTF8.GetBytes(tls.CertificatePem), "/mosquitto/certs/fullchain.pem")
            .WithResourceMapping(Encoding.UTF8.GetBytes(tls.KeyPem), "/mosquitto/certs/privkey.pem")
            // Puertos de host fijos para que un reinicio del contenedor conserve el mapeo.
            .WithPortBinding(TestServices.FreePort(), 1883)
            .WithPortBinding(TestServices.FreePort(), 8883)
            .WithWaitStrategy(Wait.ForUnixContainer().UntilInternalTcpPortIsAvailable(1883).UntilInternalTcpPortIsAvailable(8883))
            .Build();

        try
        {
            await Task.WhenAll(postgres.StartAsync(), mosquitto.StartAsync());
        }
        catch
        {
            await postgres.DisposeAsync();
            await mosquitto.DisposeAsync();
            throw;
        }

        return new ContainerTestServices(postgres, mosquitto);
    }

    public async Task StopBrokerAsync() => await _mosquitto.StopAsync();

    public async Task StartBrokerAsync() => await _mosquitto.StartAsync();

    public async ValueTask DisposeAsync()
    {
        await _mosquitto.DisposeAsync();
        await _postgres.DisposeAsync();
    }
}

/// <summary>
/// PostgreSQL y Mosquitto como procesos del sistema (Linux). Usa los binarios instalados: initdb/pg_ctl (con pgvector)
/// y mosquitto + mosquitto_ctrl con el plugin de Dynamic Security. La configuración de Mosquitto se deriva de
/// deploy/mosquitto/mosquitto.conf cambiando solo rutas y puertos.
/// </summary>
internal sealed class NativeTestServices : ITestServices
{
    private readonly string _root;
    private readonly string _pgBin;
    private readonly string _pgData;
    private readonly string _mosquittoConf;
    private readonly bool _asRoot;
    private Process? _broker;

    private NativeTestServices(string root, string pgBin, bool asRoot)
    {
        _root = root;
        _pgBin = pgBin;
        _asRoot = asRoot;
        _pgData = Path.Combine(root, "pg");
        _mosquittoConf = Path.Combine(root, "mosquitto", "mosquitto.conf");
    }

    public string Kind => "native";

    public string PostgresConnectionString { get; private set; } = string.Empty;

    public int MqttPlainPort { get; private set; }

    public int MqttTlsPort { get; private set; }

    public static bool IsAvailable(out string missing)
    {
        var problems = new List<string>();
        if (!OperatingSystem.IsLinux())
        {
            problems.Add("only on Linux");
        }

        if (FindPostgresBin() is null)
        {
            problems.Add("initdb/pg_ctl");
        }

        if (Which("mosquitto") is null || Which("mosquitto_ctrl") is null)
        {
            problems.Add("mosquitto, mosquitto_ctrl");
        }

        if (FindDynsecPlugin() is null)
        {
            problems.Add("mosquitto_dynamic_security.so");
        }

        missing = string.Join(", ", problems);
        return problems.Count == 0;
    }

    public static async Task<NativeTestServices> StartAsync(string stateDirectory, string adminPassword, TlsMaterial tls)
    {
        if (!IsAvailable(out var missing))
        {
            throw new InvalidOperationException($"Native test services unavailable: {missing}");
        }

        var root = Path.Combine(stateDirectory, "services");
        Directory.CreateDirectory(root);
        var services = new NativeTestServices(root, FindPostgresBin()!, Environment.UserName == "root");
        try
        {
            await services.StartPostgresAsync();
            await services.PrepareBrokerAsync(adminPassword, tls);
            await services.StartBrokerAsync();
        }
        catch
        {
            await services.DisposeAsync();
            throw;
        }

        return services;
    }

    private async Task StartPostgresAsync()
    {
        Directory.CreateDirectory(_pgData);
        if (_asRoot)
        {
            // initdb se niega a correr como root: el clúster pertenece al usuario postgres.
            await RunAsync("chown", ["-R", "postgres:postgres", _pgData]);
            await RunAsync("chmod", ["755", _root]);
        }

        await RunPgAsync("initdb", ["-D", _pgData, "-U", "postgres", "-A", "trust", "-E", "UTF8", "--no-instructions"]);
        var port = TestServices.FreePort();
        await RunPgAsync("pg_ctl", [
            "-D", _pgData, "-w", "-t", "60", "-l", Path.Combine(_pgData, "server.log"),
            "-o", $"-p {port} -k {_pgData} -c listen_addresses=127.0.0.1 -c fsync=off -c max_connections=200",
            "start",
        ]);
        PostgresConnectionString = $"Host=127.0.0.1;Port={port};Username=postgres;Password=postgres;Database=postgres;Pooling=true";
    }

    private async Task PrepareBrokerAsync(string adminPassword, TlsMaterial tls)
    {
        var dir = Path.GetDirectoryName(_mosquittoConf)!;
        var data = Path.Combine(dir, "data");
        var certs = Path.Combine(dir, "certs");
        Directory.CreateDirectory(data);
        Directory.CreateDirectory(certs);
        await File.WriteAllTextAsync(Path.Combine(certs, "fullchain.pem"), tls.CertificatePem);
        await File.WriteAllTextAsync(Path.Combine(certs, "privkey.pem"), tls.KeyPem);

        MqttPlainPort = TestServices.FreePort();
        MqttTlsPort = TestServices.FreePort();
        var production = await File.ReadAllTextAsync(Path.Combine(TestServices.DeployDirectory("mosquitto"), "mosquitto.conf"));
        await File.WriteAllTextAsync(_mosquittoConf, DeriveConfig(production, data, certs, MqttPlainPort, MqttTlsPort, _asRoot));
        await RunAsync("mosquitto_ctrl", ["dynsec", "init", Path.Combine(data, "dynamic-security.json"), "bootstrap-admin", adminPassword]);
    }

    /// <summary>La configuración de producción con rutas locales, puertos libres y solo loopback.</summary>
    internal static string DeriveConfig(string production, string data, string certs, int plainPort, int tlsPort, bool asRoot)
    {
        var lines = new List<string>();
        foreach (var raw in production.Split('\n'))
        {
            var line = raw.TrimEnd('\r');
            line = line.Trim() switch
            {
                var l when l.StartsWith("plugin ", StringComparison.Ordinal) => $"plugin {FindDynsecPlugin()}",
                var l when l.StartsWith("plugin_opt_config_file", StringComparison.Ordinal) => $"plugin_opt_config_file {Path.Combine(data, "dynamic-security.json")}",
                "listener 1883" => $"listener {plainPort} 127.0.0.1",
                "listener 8883" => $"listener {tlsPort} 127.0.0.1",
                var l when l.StartsWith("certfile", StringComparison.Ordinal) => $"certfile {Path.Combine(certs, "fullchain.pem")}",
                var l when l.StartsWith("keyfile", StringComparison.Ordinal) => $"keyfile {Path.Combine(certs, "privkey.pem")}",
                var l when l.StartsWith("persistence_location", StringComparison.Ordinal) => $"persistence_location {data}/",
                _ => line,
            };
            lines.Add(line);
        }

        if (asRoot)
        {
            // Como root, mosquitto bajaría al usuario "mosquitto", que no puede leer el directorio temporal.
            lines.Add("user root");
        }

        return string.Join('\n', lines) + "\n";
    }

    public async Task StartBrokerAsync()
    {
        if (_broker is { HasExited: false })
        {
            return;
        }

        var start = new ProcessStartInfo("mosquitto", ["-c", _mosquittoConf])
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        var broker = Process.Start(start) ?? throw new InvalidOperationException("mosquitto did not start");
        var log = new StringBuilder();
        broker.OutputDataReceived += (_, e) => { lock (log) { log.AppendLine(e.Data); } };
        broker.ErrorDataReceived += (_, e) => { lock (log) { log.AppendLine(e.Data); } };
        broker.BeginOutputReadLine();
        broker.BeginErrorReadLine();
        _broker = broker;

        var deadline = DateTime.UtcNow.AddSeconds(20);
        while (!(PortOpen(MqttPlainPort) && PortOpen(MqttTlsPort)))
        {
            if (broker.HasExited || DateTime.UtcNow > deadline)
            {
                string text;
                lock (log)
                {
                    text = log.ToString();
                }

                throw new InvalidOperationException($"mosquitto failed to listen: {text}");
            }

            await Task.Delay(100);
        }
    }

    public async Task StopBrokerAsync()
    {
        if (_broker is null)
        {
            return;
        }

        if (!_broker.HasExited)
        {
            // SIGTERM: mosquitto guarda la persistencia (sesiones y mensajes en cola) antes de salir.
            await RunAsync("kill", ["-TERM", _broker.Id.ToString(CultureInfo.InvariantCulture)]);
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            try
            {
                await _broker.WaitForExitAsync(timeout.Token);
            }
            catch (OperationCanceledException)
            {
                _broker.Kill(entireProcessTree: true);
            }
        }

        _broker.Dispose();
        _broker = null;
    }

    public async ValueTask DisposeAsync()
    {
        try
        {
            await StopBrokerAsync();
        }
        catch (InvalidOperationException)
        {
        }

        if (File.Exists(Path.Combine(_pgData, "PG_VERSION")))
        {
            try
            {
                await RunPgAsync("pg_ctl", ["-D", _pgData, "-m", "immediate", "-w", "stop"]);
            }
            catch (InvalidOperationException)
            {
            }
        }
    }

    private Task RunPgAsync(string tool, IReadOnlyList<string> arguments)
    {
        var path = Path.Combine(_pgBin, tool);
        return _asRoot
            ? RunAsync("runuser", ["-u", "postgres", "--", path, .. arguments])
            : RunAsync(path, arguments);
    }

    private static async Task RunAsync(string file, IReadOnlyList<string> arguments)
    {
        var start = new ProcessStartInfo(file, arguments)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        using var process = Process.Start(start) ?? throw new InvalidOperationException($"{file} did not start");
        var stdout = process.StandardOutput.ReadToEndAsync();
        var stderr = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"{file} exited with {process.ExitCode}: {await stderr} {await stdout}");
        }
    }

    private static bool PortOpen(int port)
    {
        try
        {
            using var client = new TcpClient();
            client.Connect(IPAddress.Loopback, port);
            return true;
        }
        catch (SocketException)
        {
            return false;
        }
    }

    private static string? FindPostgresBin()
    {
        var configured = Environment.GetEnvironmentVariable("JARVIS_TEST_PG_BIN");
        if (!string.IsNullOrWhiteSpace(configured))
        {
            return File.Exists(Path.Combine(configured, "initdb")) ? configured : null;
        }

        const string root = "/usr/lib/postgresql";
        if (Directory.Exists(root))
        {
            var newest = Directory.GetDirectories(root)
                .Select(d => (Dir: d, Version: int.TryParse(Path.GetFileName(d), NumberStyles.Integer, CultureInfo.InvariantCulture, out var v) ? v : 0))
                .Where(x => File.Exists(Path.Combine(x.Dir, "bin", "initdb")))
                .OrderByDescending(x => x.Version)
                .FirstOrDefault();
            if (newest.Dir is not null)
            {
                return Path.Combine(newest.Dir, "bin");
            }
        }

        var initdb = Which("initdb");
        return initdb is null ? null : Path.GetDirectoryName(initdb);
    }

    private static string? FindDynsecPlugin()
    {
        string[] candidates =
        [
            "/usr/lib/x86_64-linux-gnu/mosquitto_dynamic_security.so",
            "/usr/lib/aarch64-linux-gnu/mosquitto_dynamic_security.so",
            "/usr/lib/mosquitto_dynamic_security.so",
            "/usr/local/lib/mosquitto_dynamic_security.so",
        ];
        return candidates.FirstOrDefault(File.Exists);
    }

    private static string? Which(string name) =>
        (Environment.GetEnvironmentVariable("PATH") ?? string.Empty)
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Select(dir => Path.Combine(dir, name))
            .FirstOrDefault(File.Exists);
}
