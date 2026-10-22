using System.Collections.Concurrent;
using System.Text.Json;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Microsoft.Data.Sqlite;

namespace Jarvis.Simulators;

public sealed record WindowsSimulatorOptions
{
    public TimeSpan ExecutionDelay { get; init; } = TimeSpan.FromSeconds(1);

    public int DropAcks { get; init; }

    public bool DisconnectBeforeAck { get; init; }

    public bool CrashAfterStart { get; init; }

    public string? FailWith { get; init; }

    public IReadOnlySet<string> RejectTypes { get; init; } = new HashSet<string>();

    public bool Busy { get; init; }

    /// <summary>Tipos que el device deja esperando aprobación en vez de ejecutarlos (HU-057).</summary>
    public IReadOnlySet<string> RequireApprovalTypes { get; init; } = new HashSet<string>();

    public TimeSpan OutboxResendInterval { get; init; } = TimeSpan.FromSeconds(60);
}

/// <summary>
/// HU-048: device ejecutor simulado que cumple W1-W10 del Windows Agent Contract. Nunca ejecuta dos veces el mismo
/// taskId: inbox/outbox y contador de ejecuciones persistidos en SQLite (sobreviven reinicios del simulador).
/// </summary>
public sealed class WindowsSimulator : IAsyncDisposable
{
    public static readonly IReadOnlyList<string> SupportedTypes = ["OPEN_CHROME"];

    private readonly WindowsSimulatorOptions _options;
    private readonly LocalTaskStore _store;
    private readonly Action<string> _log;
    private readonly ConcurrentDictionary<Guid, CancellationTokenSource> _running = new();
    private readonly ConcurrentDictionary<Guid, Guid?> _pendingApproval = new();
    private readonly ConcurrentDictionary<Guid, string> _types = new();
    private readonly ConcurrentDictionary<Guid, SignatureInfo?> _dispatchSignatures = new();
    private readonly List<TaskApprovalDecisionData> _approvalDecisions = [];
    private readonly CancellationTokenSource _lifetime = new();
    private int _acksToDrop;
    private bool _disconnectPending;
    private Task? _resendLoop;

    public WindowsSimulator(DeviceSettings settings, string stateDirectory, WindowsSimulatorOptions options, Action<string> log)
    {
        _options = options;
        _log = log;
        _acksToDrop = options.DropAcks;
        _disconnectPending = options.DisconnectBeforeAck;
        _store = new LocalTaskStore(Path.Combine(stateDirectory, $"windows-{settings.DeviceId:N}.db"));
        Connection = new DeviceConnection(
            settings with
            {
                Capabilities = ["agent", "windows-control", "notifications"],
                SupportedTaskTypes = SupportedTypes,
                Platform = "windows",
                SoftwareVersion = "windows-simulator/0.1.0",
            },
            _store.OpenTaskIds,
            () => options.Busy || !_running.IsEmpty,
            m => log($"[windows] {m}"));
        Connection.MessageReceived += OnMessageAsync;
        Connection.Ready += OnReadyAsync;
    }

    public DeviceConnection Connection { get; }

    public int ExecutionCount(Guid taskId) => _store.ExecutionCount(taskId);

    public string? LocalStatus(Guid taskId) => _store.Get(taskId)?.Status;

    public bool IsSettled(Guid taskId) => _store.Get(taskId)?.Settled == true;

    public void Start()
    {
        RecoverInterruptedExecutions();
        Connection.Start();
        _resendLoop = Task.Run(async () =>
        {
            while (!_lifetime.IsCancellationRequested)
            {
                await Task.Delay(_options.OutboxResendInterval, _lifetime.Token).ContinueWith(_ => { }, TaskScheduler.Default);
                if (Connection.IsConnected)
                {
                    await ResendUnsettledAsync(terminalOnly: true);
                }
            }
        });
    }

    public async Task StopAsync()
    {
        await _lifetime.CancelAsync();
        await Connection.StopAsync();
    }

    public async ValueTask DisposeAsync()
    {
        await _lifetime.CancelAsync();
        if (_resendLoop is not null)
        {
            await _resendLoop.ContinueWith(_ => { }, TaskScheduler.Default);
        }

        await Connection.DisposeAsync();
        _lifetime.Dispose();
        _store.Dispose();
    }

    /// <summary>W9: una tarea "iniciada" sin resultado tras un reinicio se reporta como fallida, nunca se reejecuta.</summary>
    private void RecoverInterruptedExecutions()
    {
        foreach (var task in _store.InStatus("Starting"))
        {
            var envelope = MqttEnvelope.Create(
                MessageTypes.TaskFailed,
                Connection.DeviceId,
                new TaskFailedData(task.TaskId, new ErrorInfo(ErrorCodes.ExecutionError, "agent restarted", false)),
                DateTimeOffset.UtcNow,
                task.CorrelationId);
            _store.Update(task.TaskId, "Failed", envelope);
            _log($"[windows] task {task.TaskId} was interrupted by a restart; reporting EXECUTION_ERROR");
        }
    }

    private async Task OnReadyAsync()
    {
        foreach (var received in _store.InStatus("Received"))
        {
            await AcknowledgeOrDropAsync(received.TaskId, received.CorrelationId);
        }

        await ResendUnsettledAsync(terminalOnly: false, skipStatus: "Received");

        // Entregadas cuya ejecución no llegó a empezar antes de un reinicio: se ejecutan (una sola vez).
        foreach (var delivered in _store.InStatus("Delivered").Where(t => !_running.ContainsKey(t.TaskId) && _store.ExecutionCount(t.TaskId) == 0))
        {
            StartExecution(delivered.TaskId, delivered.CorrelationId);
        }
    }

    private async Task ResendUnsettledAsync(bool terminalOnly, string? skipStatus = null)
    {
        foreach (var task in _store.Unsettled())
        {
            if (task.LastMessage is null || (terminalOnly && !IsTerminal(task.Status)) || task.Status == skipStatus || task.Status == "Delivered" && _running.ContainsKey(task.TaskId))
            {
                continue;
            }

            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, MqttEnvelope.Parse(System.Text.Encoding.UTF8.GetBytes(task.LastMessage)));
        }
    }

    private async Task OnMessageAsync(MqttEnvelope envelope)
    {
        switch (envelope.Type)
        {
            case MessageTypes.TaskDispatch:
                await OnDispatchAsync(envelope, envelope.ReadData<TaskDispatchData>());
                break;
            case MessageTypes.TaskCancel:
                await OnCancelAsync(envelope, envelope.ReadData<TaskCancelData>());
                break;
            case MessageTypes.TaskUpdateAck:
                var ack = envelope.ReadData<TaskUpdateAckData>();
                _store.MarkSettled(ack.TaskId);
                _log($"[windows] task {ack.TaskId} settled ({ack.Status})");
                break;
            case MessageTypes.TaskApprovalDecision:
                await OnApprovalDecisionAsync(envelope, envelope.ReadData<TaskApprovalDecisionData>());
                break;
        }
    }

    /// <summary>
    /// Decisiones de aprobación recibidas, para que las pruebas comprueben qué llegó.
    /// El simulador <b>no verifica la firma</b>: eso lo hace el agente real (HU-WIN-062), y las
    /// pruebas de este repositorio no deben dar a entender lo contrario.
    /// </summary>
    public IReadOnlyList<TaskApprovalDecisionData> ApprovalDecisions
    {
        get
        {
            lock (_approvalDecisions)
            {
                return [.. _approvalDecisions];
            }
        }
    }

    /// <summary>Firmas recibidas en los dispatch, por taskId: prueba que el Cloud las transmite íntegras.</summary>
    public IReadOnlyDictionary<Guid, SignatureInfo?> DispatchSignatures => _dispatchSignatures;

    private async Task OnApprovalDecisionAsync(MqttEnvelope envelope, TaskApprovalDecisionData data)
    {
        lock (_approvalDecisions)
        {
            _approvalDecisions.Add(data);
        }

        var waiting = _pendingApproval.TryRemove(data.TaskId, out var correlationId);
        _log($"[windows] approval decision {data.Decision} for {data.TaskId} (key {data.Signature.KeyId}, waiting={waiting})");
        if (!waiting)
        {
            return;
        }

        if (data.Decision == "GRANTED")
        {
            StartExecution(data.TaskId, correlationId);
            return;
        }

        var cancelled = MqttEnvelope.Create(
            MessageTypes.TaskCancelled,
            Connection.DeviceId,
            new TaskCancelledData(data.TaskId, "APPROVAL_DENIED"),
            DateTimeOffset.UtcNow,
            correlationId);
        _store.Update(data.TaskId, "Cancelled", cancelled);
        await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, cancelled);
    }

    private async Task OnDispatchAsync(MqttEnvelope envelope, TaskDispatchData data)
    {
        var known = _store.Get(data.TaskId);
        if (known is { Status: "Received" })
        {
            // Recibida y persistida pero sin ack publicado todavía (flags de prueba).
            await AcknowledgeOrDropAsync(data.TaskId, known.CorrelationId);
            return;
        }

        if (known is not null)
        {
            // W4: taskId conocido → nunca se ejecuta de nuevo; se reenvía el último estado.
            _log($"[windows] duplicate dispatch of {data.TaskId} (attempt {data.Attempt}); resending last state {known.Status}");
            if (known.LastMessage is not null)
            {
                await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, MqttEnvelope.Parse(System.Text.Encoding.UTF8.GetBytes(known.LastMessage)));
            }

            return;
        }

        var correlationId = envelope.CorrelationId;
        if (DateTimeOffset.UtcNow > data.ExpiresAt)
        {
            await RejectAsync(data.TaskId, correlationId, ErrorCodes.ExpiredOnArrival, "Task expired before arrival.");
            return;
        }

        if (!SupportedTypes.Contains(data.Type))
        {
            await RejectAsync(data.TaskId, correlationId, ErrorCodes.UnsupportedTaskType, $"{data.Type} is not supported.");
            return;
        }

        if (_options.RejectTypes.Contains(data.Type))
        {
            await RejectAsync(data.TaskId, correlationId, ErrorCodes.NotPermitted, "Task type not allowed by local policy.");
            return;
        }

        if (data.Type == "OPEN_CHROME" && !IsAllowedUrl(data.Payload))
        {
            await RejectAsync(data.TaskId, correlationId, ErrorCodes.InvalidPayload, "payload.url must be an https URL.");
            return;
        }

        _dispatchSignatures[data.TaskId] = data.Signature;
        _types[data.TaskId] = data.Type;

        // W5: persistir antes de confirmar.
        var ackEnvelope = MqttEnvelope.Create(MessageTypes.TaskAck, Connection.DeviceId, new TaskRefData(data.TaskId), DateTimeOffset.UtcNow, correlationId);
        _store.Insert(data.TaskId, correlationId, "Received", ackEnvelope);

        if (_disconnectPending)
        {
            _disconnectPending = false;
            _log($"[windows] simulating connection loss before ack of {data.TaskId}");
            await Connection.DropConnectionAsync();
            return;
        }

        await AcknowledgeOrDropAsync(data.TaskId, correlationId);
    }

    /// <summary>Publica el ack (salvo que un flag de prueba lo descarte) y empieza la ejecución una única vez.</summary>
    private async Task AcknowledgeOrDropAsync(Guid taskId, Guid? correlationId)
    {
        if (Interlocked.Decrement(ref _acksToDrop) >= 0)
        {
            _log($"[windows] dropping ack of {taskId} (test flag)");
            return;
        }

        var ackEnvelope = MqttEnvelope.Create(MessageTypes.TaskAck, Connection.DeviceId, new TaskRefData(taskId), DateTimeOffset.UtcNow, correlationId);
        _store.Update(taskId, "Delivered", ackEnvelope);
        await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, ackEnvelope);

        var type = _types.GetValueOrDefault(taskId);
        if (type is not null && _options.RequireApprovalTypes.Contains(type))
        {
            _pendingApproval[taskId] = correlationId;
            var required = MqttEnvelope.Create(
                MessageTypes.TaskApprovalRequired,
                Connection.DeviceId,
                new TaskApprovalRequiredData(taskId, "Local policy requires approval (simulated R3)"),
                DateTimeOffset.UtcNow,
                correlationId);
            _store.Update(taskId, "WaitingForApproval", required);
            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, required);
            _log($"[windows] {taskId} waiting for approval");
            return;
        }

        StartExecution(taskId, correlationId);
    }

    private void StartExecution(Guid taskId, Guid? correlationId)
    {
        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_lifetime.Token);
        if (!_running.TryAdd(taskId, cancellation))
        {
            cancellation.Dispose();
            return;
        }

        _ = Task.Run(() => ExecuteAsync(taskId, correlationId, cancellation));
    }

    private async Task ExecuteAsync(Guid taskId, Guid? correlationId, CancellationTokenSource cancellation)
    {
        try
        {
            var started = MqttEnvelope.Create(MessageTypes.TaskStarted, Connection.DeviceId, new TaskStartedData(taskId, DateTimeOffset.UtcNow), DateTimeOffset.UtcNow, correlationId);
            _store.Update(taskId, "Starting", started);
            _store.IncrementExecutions(taskId);
            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, started);
            _log($"[windows] executing {taskId} (simulated)");

            if (_options.CrashAfterStart)
            {
                _log("[windows] simulating crash after start");
                await Connection.CrashAsync();
                return;
            }

            await Task.Delay(_options.ExecutionDelay, cancellation.Token);

            MqttEnvelope final = _options.FailWith is { } code
                ? MqttEnvelope.Create(MessageTypes.TaskFailed, Connection.DeviceId, new TaskFailedData(taskId, new ErrorInfo(code, "Simulated failure", false)), DateTimeOffset.UtcNow, correlationId)
                : MqttEnvelope.Create(
                    MessageTypes.TaskCompleted,
                    Connection.DeviceId,
                    new TaskCompletedData(taskId, DateTimeOffset.UtcNow, JarvisJson.ToElement(new { outcome = "SUCCESS", message = "Chrome opened (simulated)" })),
                    DateTimeOffset.UtcNow,
                    correlationId);
            _store.Update(taskId, _options.FailWith is null ? "Completed" : "Failed", final);
            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, final);
            _log($"[windows] task {taskId} finished: {(_options.FailWith is null ? "SUCCESS" : _options.FailWith)}");
        }
        catch (OperationCanceledException) when (!_lifetime.IsCancellationRequested)
        {
            var cancelled = MqttEnvelope.Create(MessageTypes.TaskCancelled, Connection.DeviceId, new TaskCancelledData(taskId, ErrorCodes.UserRequest), DateTimeOffset.UtcNow, correlationId);
            _store.Update(taskId, "Cancelled", cancelled);
            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, cancelled);
            _log($"[windows] task {taskId} cancelled");
        }
        catch (OperationCanceledException)
        {
            // El simulador se detiene; al reiniciar la tarea se reportará como interrumpida.
        }
        finally
        {
            _running.TryRemove(taskId, out _);
            cancellation.Dispose();
        }
    }

    private async Task OnCancelAsync(MqttEnvelope envelope, TaskCancelData data)
    {
        if (_running.TryGetValue(data.TaskId, out var running))
        {
            await running.CancelAsync();
            return;
        }

        var known = _store.Get(data.TaskId);
        if (known?.LastMessage is not null && IsTerminal(known.Status))
        {
            await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, MqttEnvelope.Parse(System.Text.Encoding.UTF8.GetBytes(known.LastMessage)));
            return;
        }

        // W8: tarea desconocida o aún no iniciada → queda registrada como cancelada para rechazar un dispatch posterior.
        var cancelled = MqttEnvelope.Create(MessageTypes.TaskCancelled, Connection.DeviceId, new TaskCancelledData(data.TaskId, ErrorCodes.UserRequest), DateTimeOffset.UtcNow, envelope.CorrelationId);
        if (known is null)
        {
            _store.Insert(data.TaskId, envelope.CorrelationId, "Cancelled", cancelled);
        }
        else
        {
            _store.Update(data.TaskId, "Cancelled", cancelled);
        }

        await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, cancelled);
    }

    private async Task RejectAsync(Guid taskId, Guid? correlationId, string code, string message)
    {
        var rejected = MqttEnvelope.Create(MessageTypes.TaskRejected, Connection.DeviceId, new TaskRejectedData(taskId, new ErrorInfo(code, message, false)), DateTimeOffset.UtcNow, correlationId);
        _store.Insert(taskId, correlationId, "Rejected", rejected);
        await Connection.PublishEnvelopeAsync(TopicChannels.Tasks, rejected);
        _log($"[windows] task {taskId} rejected: {code}");
    }

    private static bool IsAllowedUrl(JsonElement payload) =>
        !payload.TryGetProperty("url", out var url)
        || (url.ValueKind == JsonValueKind.String && Uri.TryCreate(url.GetString(), UriKind.Absolute, out var uri) && uri.Scheme == Uri.UriSchemeHttps);

    private static bool IsTerminal(string status) => status is "Completed" or "Failed" or "Cancelled" or "Rejected";
}

public sealed record LocalTask(Guid TaskId, Guid? CorrelationId, string Status, string? LastMessage, bool Settled);

/// <summary>Inbox/outbox local del simulador (equivalente al SQLite del futuro Windows Agent).</summary>
public sealed class LocalTaskStore : IDisposable
{
    private readonly SqliteConnection _connection;
    private readonly Lock _lock = new();

    public LocalTaskStore(string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(path))!);
        _connection = new SqliteConnection($"Data Source={path}");
        _connection.Open();
        Execute("""
            CREATE TABLE IF NOT EXISTS tasks (
              task_id TEXT PRIMARY KEY,
              correlation_id TEXT NULL,
              status TEXT NOT NULL,
              last_message TEXT NULL,
              settled INTEGER NOT NULL DEFAULT 0,
              executions INTEGER NOT NULL DEFAULT 0,
              received_at TEXT NOT NULL);
            """);
        // W7: se conservan ≥ 8 días.
        Execute("DELETE FROM tasks WHERE settled = 1 AND received_at < $cutoff", ("$cutoff", DateTimeOffset.UtcNow.AddDays(-8).ToString("O")));
    }

    public void Insert(Guid taskId, Guid? correlationId, string status, MqttEnvelope lastMessage) =>
        Execute(
            "INSERT OR IGNORE INTO tasks (task_id, correlation_id, status, last_message, received_at) VALUES ($id, $c, $s, $m, $t)",
            ("$id", taskId.ToString()),
            ("$c", (object?)correlationId?.ToString() ?? DBNull.Value),
            ("$s", status),
            ("$m", System.Text.Encoding.UTF8.GetString(lastMessage.ToBytes())),
            ("$t", DateTimeOffset.UtcNow.ToString("O")));

    public void Update(Guid taskId, string status, MqttEnvelope lastMessage) =>
        Execute(
            "UPDATE tasks SET status = $s, last_message = $m WHERE task_id = $id",
            ("$id", taskId.ToString()),
            ("$s", status),
            ("$m", System.Text.Encoding.UTF8.GetString(lastMessage.ToBytes())));

    public void IncrementExecutions(Guid taskId) =>
        Execute("UPDATE tasks SET executions = executions + 1 WHERE task_id = $id", ("$id", taskId.ToString()));

    public void MarkSettled(Guid taskId) => Execute("UPDATE tasks SET settled = 1 WHERE task_id = $id", ("$id", taskId.ToString()));

    public int ExecutionCount(Guid taskId)
    {
        lock (_lock)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = "SELECT executions FROM tasks WHERE task_id = $id";
            command.Parameters.AddWithValue("$id", taskId.ToString());
            return command.ExecuteScalar() is long count ? (int)count : 0;
        }
    }

    public LocalTask? Get(Guid taskId) => Query("SELECT task_id, correlation_id, status, last_message, settled FROM tasks WHERE task_id = $id", ("$id", taskId.ToString())).FirstOrDefault();

    public IReadOnlyList<LocalTask> Unsettled() => Query("SELECT task_id, correlation_id, status, last_message, settled FROM tasks WHERE settled = 0 ORDER BY received_at");

    public IReadOnlyList<LocalTask> InStatus(string status) => Query("SELECT task_id, correlation_id, status, last_message, settled FROM tasks WHERE status = $s", ("$s", status));

    /// <summary>openTaskIds: no terminales y terminales aún sin task.update.ack (máx. 256).</summary>
    public IReadOnlyList<Guid> OpenTaskIds() => Unsettled().Select(t => t.TaskId).Take(256).ToList();

    public void Dispose() => _connection.Dispose();

    private void Execute(string sql, params (string Name, object Value)[] parameters)
    {
        lock (_lock)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = sql;
            foreach (var (name, value) in parameters)
            {
                command.Parameters.AddWithValue(name, value);
            }

            command.ExecuteNonQuery();
        }
    }

    private List<LocalTask> Query(string sql, params (string Name, object Value)[] parameters)
    {
        lock (_lock)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = sql;
            foreach (var (name, value) in parameters)
            {
                command.Parameters.AddWithValue(name, value);
            }

            using var reader = command.ExecuteReader();
            var result = new List<LocalTask>();
            while (reader.Read())
            {
                result.Add(new LocalTask(
                    Guid.Parse(reader.GetString(0)),
                    reader.IsDBNull(1) ? null : Guid.Parse(reader.GetString(1)),
                    reader.GetString(2),
                    reader.IsDBNull(3) ? null : reader.GetString(3),
                    reader.GetInt64(4) == 1));
            }

            return result;
        }
    }
}
