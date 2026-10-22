using System.Globalization;
using Jarvis.Simulators;

// jarvis-sim windows|esp32 --device-id <id> --secret-file <f> [--host h] [--port p] [--no-tls] [--ca-file f] [--state-dir d] ...
if (args.Length == 0 || args[0] is not ("windows" or "esp32"))
{
    Console.Error.WriteLine("""
        Usage:
          jarvis-sim windows --device-id <id> --secret-file <file> [common] [--delay <s>] [--drop-acks <n>] [--disconnect-before-ack]
                             [--crash-after-start] [--pause-heartbeats <s>] [--busy] [--fail-with <CODE>] [--slow <s>] [--reject-type <TYPE>]
          jarvis-sim esp32   --device-id <id> --secret-file <file> --target <windowsDeviceId> [common] [--press-once] [--type <TYPE>] [--url <https://...>]
        Common: [--host <host>] [--port <port>] [--no-tls] [--ca-file <pem>] [--state-dir <dir>]
        The secret can also be provided with the JARVIS_DEVICE_SECRET environment variable.
        """);
    return 2;
}

var options = ParseOptions(args[1..]);
string? Get(string name) => options.TryGetValue(name, out var value) ? value : null;
bool Has(string name) => options.ContainsKey(name);

var deviceId = Guid.Parse(Get("--device-id") ?? throw new ArgumentException("--device-id is required"));
var secret = Get("--secret-file") is { } secretFile
    ? File.ReadAllText(secretFile).Trim()
    : Environment.GetEnvironmentVariable("JARVIS_DEVICE_SECRET") ?? throw new ArgumentException("--secret-file or JARVIS_DEVICE_SECRET is required");
var useTls = !Has("--no-tls");
var settings = new DeviceSettings
{
    DeviceId = deviceId,
    Secret = secret,
    Host = Get("--host") ?? "localhost",
    Port = int.Parse(Get("--port") ?? (useTls ? "8883" : "1883"), CultureInfo.InvariantCulture),
    UseTls = useTls,
    CaFile = Get("--ca-file"),
};
var stateDirectory = Get("--state-dir") ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "jarvis-sim");
void Log(string message) => Console.WriteLine($"{DateTimeOffset.Now:HH:mm:ss} {message}");

using var shutdown = new CancellationTokenSource();
Console.CancelKeyPress += (_, e) =>
{
    e.Cancel = true;
    shutdown.Cancel();
};

if (args[0] == "windows")
{
    var windowsOptions = new WindowsSimulatorOptions
    {
        ExecutionDelay = TimeSpan.FromSeconds(double.Parse(Get("--slow") ?? Get("--delay") ?? "1", CultureInfo.InvariantCulture)),
        DropAcks = int.Parse(Get("--drop-acks") ?? "0", CultureInfo.InvariantCulture),
        DisconnectBeforeAck = Has("--disconnect-before-ack"),
        CrashAfterStart = Has("--crash-after-start"),
        FailWith = Get("--fail-with"),
        RejectTypes = Get("--reject-type") is { } reject ? new HashSet<string> { reject } : new HashSet<string>(),
        Busy = Has("--busy"),
    };
    await using var simulator = new WindowsSimulator(settings, stateDirectory, windowsOptions, Log);
    simulator.Start();
    if (Get("--pause-heartbeats") is { } pause)
    {
        _ = Task.Run(async () =>
        {
            await simulator.Connection.WaitReadyAsync(TimeSpan.FromMinutes(5));
            simulator.Connection.PauseHeartbeats = true;
            Log($"[windows] heartbeats paused for {pause}s");
            await Task.Delay(TimeSpan.FromSeconds(double.Parse(pause, CultureInfo.InvariantCulture)));
            simulator.Connection.PauseHeartbeats = false;
        });
    }

    Log("[windows] running; Ctrl+C to exit");
    await Task.Delay(Timeout.Infinite, shutdown.Token).ContinueWith(_ => { }, TaskScheduler.Default);
    await simulator.StopAsync();
    return 0;
}

var target = Guid.Parse(Get("--target") ?? throw new ArgumentException("--target is required"));
await using var esp32 = new Esp32Simulator(
    settings,
    stateDirectory,
    new Esp32SimulatorOptions { TargetDeviceId = target, TaskType = Get("--type") ?? "OPEN_CHROME", Url = Get("--url") ?? "https://example.com" },
    Log);
esp32.Start();
await esp32.Connection.WaitReadyAsync(TimeSpan.FromSeconds(60));

if (Has("--press-once"))
{
    var result = await esp32.PressButtonAsync();
    if (!result.Accepted)
    {
        await esp32.StopAsync();
        return 1;
    }

    var final = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
    esp32.TaskUpdated += update =>
    {
        if (update.TaskId == result.TaskId && update.Status is "Completed" or "Failed" or "Cancelled" or "Expired")
        {
            final.TrySetResult(update.Status);
        }
    };
    var status = await final.Task.WaitAsync(TimeSpan.FromMinutes(2));
    await esp32.StopAsync();
    return status == "Completed" ? 0 : 1;
}

Log("[esp32] press 'b' to press the button, 'q' to quit");
while (!shutdown.IsCancellationRequested)
{
    if (!Console.KeyAvailable)
    {
        await Task.Delay(100).ContinueWith(_ => { }, TaskScheduler.Default);
        continue;
    }

    var key = Console.ReadKey(intercept: true).KeyChar;
    if (key == 'q')
    {
        break;
    }

    if (key == 'b')
    {
        await esp32.PressButtonAsync();
    }
}

await esp32.StopAsync();
return 0;

static Dictionary<string, string> ParseOptions(string[] args)
{
    var result = new Dictionary<string, string>(StringComparer.Ordinal);
    for (var i = 0; i < args.Length; i++)
    {
        if (!args[i].StartsWith("--", StringComparison.Ordinal))
        {
            continue;
        }

        var hasValue = i + 1 < args.Length && !args[i + 1].StartsWith("--", StringComparison.Ordinal);
        result[args[i]] = hasValue ? args[++i] : "true";
    }

    return result;
}
