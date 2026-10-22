using System.Collections.Concurrent;
using System.Globalization;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;

namespace Jarvis.Simulators;

public sealed record Esp32SimulatorOptions
{
    public required Guid TargetDeviceId { get; init; }

    public string TaskType { get; init; } = "OPEN_CHROME";

    public string? Url { get; init; } = "https://example.com";

    public int ExpiresInSeconds { get; init; } = 300;

    public TimeSpan AcceptTimeout { get; init; } = TimeSpan.FromSeconds(10);

    public int MaxRequestAttempts { get; init; } = 3;
}

public sealed record ButtonPressResult(bool Accepted, Guid? TaskId, bool Duplicate, string? ErrorCode, string IdempotencyKey);

/// <summary>
/// HU-049: device origen simulado (ESP32 Contract). Solo MQTT: crea tareas con <c>task.create</c>, muestra
/// <c>task.update</c> como lo haría la pantalla y consulta <c>task.get</c> al reconectar.
/// </summary>
public sealed class Esp32Simulator : IAsyncDisposable
{
    private readonly Esp32SimulatorOptions _options;
    private readonly string _counterFile;
    private readonly string _lastTaskFile;
    private readonly Action<string> _log;
    private readonly ConcurrentDictionary<Guid, TaskCompletionSource<MqttEnvelope>> _pendingRequests = new();
    private readonly ConcurrentDictionary<Guid, TaskUpdateData> _latest = new();

    public Esp32Simulator(DeviceSettings settings, string stateDirectory, Esp32SimulatorOptions options, Action<string> log)
    {
        _options = options;
        _log = log;
        Directory.CreateDirectory(stateDirectory);
        _counterFile = Path.Combine(stateDirectory, $"esp32-{settings.DeviceId:N}.counter");
        _lastTaskFile = Path.Combine(stateDirectory, $"esp32-{settings.DeviceId:N}.last-task");
        Connection = new DeviceConnection(
            settings with
            {
                Capabilities = ["task-requester", "buttons", "display"],
                SupportedTaskTypes = [],
                Platform = "esp32",
                SoftwareVersion = "esp32-simulator/0.1.0",
            },
            () => [],
            () => false,
            m => log($"[esp32] {m}"));
        Connection.MessageReceived += OnMessageAsync;
        Connection.Ready += OnReadyAsync;
    }

    public DeviceConnection Connection { get; }

    /// <summary>Se dispara con cada <c>task.update</c> recibido (lo que mostraría la pantalla).</summary>
    public event Action<TaskUpdateData>? TaskUpdated;

    public TaskUpdateData? LatestUpdate(Guid taskId) => _latest.TryGetValue(taskId, out var update) ? update : null;

    public void Start() => Connection.Start();

    public Task StopAsync() => Connection.StopAsync();

    public ValueTask DisposeAsync() => Connection.DisposeAsync();

    /// <summary>Pulsación de botón: idempotencyKey persistida ANTES de publicar; reintento con la misma key.</summary>
    public async Task<ButtonPressResult> PressButtonAsync(string? overrideKey = null)
    {
        var key = overrideKey ?? $"btn1-{NextCounter():D6}";
        var payload = _options.Url is null ? JarvisJson.EmptyObject : JarvisJson.ToElement(new { url = _options.Url });
        var data = new TaskCreateData(key, _options.TargetDeviceId, _options.TaskType, payload, _options.ExpiresInSeconds, null);

        for (var attempt = 1; attempt <= _options.MaxRequestAttempts; attempt++)
        {
            var envelope = MqttEnvelope.Create(MessageTypes.TaskCreate, Connection.DeviceId, data, DateTimeOffset.UtcNow, Guid.CreateVersion7());
            var completion = new TaskCompletionSource<MqttEnvelope>(TaskCreationOptions.RunContinuationsAsynchronously);
            _pendingRequests[envelope.Id!.Value] = completion;
            _log($"[esp32] button pressed → task.create {_options.TaskType} ({key}, attempt {attempt})");
            await Connection.PublishEnvelopeAsync(TopicChannels.Requests, envelope);
            try
            {
                var response = await completion.Task.WaitAsync(_options.AcceptTimeout);
                if (response.Type == MessageTypes.RequestAccepted)
                {
                    var accepted = response.ReadData<RequestAcceptedData>();
                    await File.WriteAllTextAsync(_lastTaskFile, accepted.TaskId.ToString());
                    _log($"[esp32] display: \"Enviado\" (task {accepted.TaskId}{(accepted.Duplicate ? ", duplicate" : string.Empty)})");
                    return new ButtonPressResult(true, accepted.TaskId, accepted.Duplicate, null, key);
                }

                var rejected = response.ReadData<RequestRejectedData>();
                _log($"[esp32] display: \"Error {rejected.Error.Code}\"");
                return new ButtonPressResult(false, null, false, rejected.Error.Code, key);
            }
            catch (TimeoutException)
            {
                _log("[esp32] no response; retrying with the same idempotency key");
            }
            finally
            {
                _pendingRequests.TryRemove(envelope.Id!.Value, out _);
            }
        }

        return new ButtonPressResult(false, null, false, "TIMEOUT", key);
    }

    public Task RequestTaskAsync(Guid taskId) =>
        Connection.PublishAsync(TopicChannels.Requests, MessageTypes.TaskGet, new TaskGetData(taskId));

    public Task CancelTaskAsync(Guid taskId) =>
        Connection.PublishAsync(TopicChannels.Requests, MessageTypes.TaskCancelRequest, new TaskCancelRequestData(taskId));

    private async Task OnReadyAsync()
    {
        if (File.Exists(_lastTaskFile) && Guid.TryParse(await File.ReadAllTextAsync(_lastTaskFile), out var last))
        {
            await RequestTaskAsync(last);
        }
    }

    private Task OnMessageAsync(MqttEnvelope envelope)
    {
        switch (envelope.Type)
        {
            case MessageTypes.RequestAccepted or MessageTypes.RequestRejected:
                if (envelope.CausationId is { } causation && _pendingRequests.TryGetValue(causation, out var completion))
                {
                    completion.TrySetResult(envelope);
                }

                break;
            case MessageTypes.TaskUpdate:
                var update = envelope.ReadData<TaskUpdateData>();
                _latest[update.TaskId] = update;
                var detail = update.Result is { } result && result.TryGetProperty("message", out var message) ? $" — {message.GetString()}" : string.Empty;
                var error = update.Error is { } e ? $" — {e.Code}" : string.Empty;
                _log($"[esp32] display: {update.Status}{detail}{error}");
                TaskUpdated?.Invoke(update);
                break;
        }

        return Task.CompletedTask;
    }

    private int NextCounter()
    {
        var current = File.Exists(_counterFile) && int.TryParse(File.ReadAllText(_counterFile), NumberStyles.None, CultureInfo.InvariantCulture, out var value) ? value : 0;
        var next = current + 1;
        File.WriteAllText(_counterFile, next.ToString(CultureInfo.InvariantCulture));
        return next;
    }
}
