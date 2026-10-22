using Jarvis.Domain.Common;
using Jarvis.Domain.Events;

namespace Jarvis.Domain.Tasks;

public enum JarvisTaskStatus
{
    Pending,
    Dispatched,
    Delivered,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

/// <summary>Resultado de aplicar un mensaje del device a la máquina de estados.</summary>
public enum TaskUpdateOutcome
{
    Applied,
    Duplicate,
    Stale,
    Terminal,
    SecurityViolation,
}

public enum CancelOutcome
{
    CancelledImmediately,
    CancelRequested,
    AlreadyRequested,
}

public sealed record TaskError(string Code, string? Message);

/// <summary>
/// Firma Ed25519 del cliente que originó la tarea (ADR-012). El Cloud la guarda y la retransmite
/// tal cual: no la produce ni la verifica, porque no tiene la clave pública. Solo comprueba su forma.
/// </summary>
public sealed record TaskSignature(string Alg, string KeyId, string Value)
{
    public static TaskSignature Create(string? alg, string? keyId, string? value)
    {
        if (!DomainRules.IsWellFormedSignature(alg, keyId, value))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Malformed signature.");
        }

        return new TaskSignature(DomainRules.SignatureAlgorithm, keyId!, value!);
    }
}

/// <summary>
/// Tarea estructurada dirigida a un device. Implementa docs/02-domain/task-state-machine.md.
/// El nombre evita colisión con System.Threading.Tasks.Task.
/// </summary>
public sealed class JarvisTask : Entity
{
    public static readonly TimeSpan DispatchedExpiryGrace = TimeSpan.FromSeconds(30);
    public static readonly TimeSpan SystemMaxDuration = TimeSpan.FromHours(24);

    private JarvisTask()
    {
    }

    public Guid OwnerId { get; private set; }

    public ActorType RequestedByType { get; private set; }

    public Guid? RequestedByUserId { get; private set; }

    public Guid? SourceDeviceId { get; private set; }

    public Guid TargetDeviceId { get; private set; }

    public string Type { get; private set; } = string.Empty;

    /// <summary>JSON (jsonb) opaco para el Cloud.</summary>
    public string Payload { get; private set; } = "{}";

    public JarvisTaskStatus Status { get; private set; }

    public string? StatusReason { get; private set; }

    public Guid CorrelationId { get; private set; }

    public string? IdempotencyKey { get; private set; }

    public byte[]? RequestHash { get; private set; }

    public int AttemptCount { get; private set; }

    public int MaxAttempts { get; private set; }

    public DateTimeOffset ExpiresAt { get; private set; }

    public int? MaxDurationSeconds { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public DateTimeOffset? DispatchedAt { get; private set; }

    public DateTimeOffset? DeliveredAt { get; private set; }

    public DateTimeOffset? StartedAt { get; private set; }

    /// <summary>Momento en que alcanzó cualquier estado terminal.</summary>
    public DateTimeOffset? CompletedAt { get; private set; }

    public DateTimeOffset? CancelRequestedAt { get; private set; }

    public DateTimeOffset? LastUpdateAt { get; private set; }

    public short? ProgressPercent { get; private set; }

    public string? ProgressMessage { get; private set; }

    /// <summary>JSON (jsonb) opaco, solo con <see cref="JarvisTaskStatus.Completed"/>.</summary>
    public string? Result { get; private set; }

    public string? ErrorCode { get; private set; }

    public string? ErrorMessage { get; private set; }

    public string? SignatureAlg { get; private set; }

    public string? SignatureKeyId { get; private set; }

    public string? SignatureValue { get; private set; }

    /// <summary>Firma del originador, si la trajo. Viaja íntegra hasta el device (ADR-012).</summary>
    public TaskSignature? Signature => SignatureAlg is null || SignatureKeyId is null || SignatureValue is null
        ? null
        : new TaskSignature(SignatureAlg, SignatureKeyId, SignatureValue);

    public bool IsSigned => SignatureAlg is not null;

    public bool IsTerminal => IsTerminalStatus(Status);

    public Actor Requester => RequestedByType == ActorType.Device
        ? Actor.Device(SourceDeviceId!.Value)
        : Actor.User(RequestedByUserId!.Value);

    public static bool IsTerminalStatus(JarvisTaskStatus status) =>
        status is JarvisTaskStatus.Completed or JarvisTaskStatus.Failed or JarvisTaskStatus.Cancelled or JarvisTaskStatus.Expired;

    public static int Rank(JarvisTaskStatus status) => status switch
    {
        JarvisTaskStatus.Pending => 0,
        JarvisTaskStatus.Dispatched => 1,
        JarvisTaskStatus.Delivered => 2,
        JarvisTaskStatus.Running or JarvisTaskStatus.WaitingForApproval => 3,
        _ => 4,
    };

    public static JarvisTask Create(
        Guid id,
        Guid ownerId,
        Actor requester,
        Guid targetDeviceId,
        string type,
        string payloadJson,
        Guid correlationId,
        string? idempotencyKey,
        byte[]? requestHash,
        int expiresInSeconds,
        int? maxDurationSeconds,
        string channel,
        DateTimeOffset now,
        int maxAttempts = DomainRules.DefaultMaxAttempts,
        DateTimeOffset? expiresAt = null,
        TaskSignature? signature = null)
    {
        if (!DomainRules.IsValidTaskType(type))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Invalid task type.");
        }

        if (DomainRules.Utf8Length(payloadJson) > DomainRules.MaxPayloadBytes)
        {
            throw new DomainException(DomainErrorCodes.PayloadTooLarge, "Payload exceeds 16 KiB.");
        }

        if (expiresInSeconds is < DomainRules.MinExpiresInSeconds or > DomainRules.MaxExpiresInSeconds)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "expiresInSeconds out of range.");
        }

        // Una tarea firmada trae su expiresAt absoluto porque forma parte de la cadena firmada:
        // el Cloud no puede recalcularlo sin invalidar la firma (ADR-012).
        var effectiveExpiresAt = expiresAt ?? now.AddSeconds(expiresInSeconds);
        if (effectiveExpiresAt < now.AddSeconds(DomainRules.MinExpiresInSeconds)
            || effectiveExpiresAt > now.AddSeconds(DomainRules.MaxExpiresInSeconds))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "expiresAt out of range.");
        }

        if (maxDurationSeconds is < 1 or > DomainRules.MaxDurationSeconds)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "maxDurationSeconds out of range.");
        }

        if (requester.Type == ActorType.System || requester.Id is null)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Tasks must be requested by a user or a device.");
        }

        if (requester.Type == ActorType.Device && requester.Id == targetDeviceId)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "A device cannot create tasks for itself.");
        }

        var task = new JarvisTask
        {
            Id = id,
            OwnerId = ownerId,
            RequestedByType = requester.Type,
            RequestedByUserId = requester.Type == ActorType.User ? requester.Id : null,
            SourceDeviceId = requester.Type == ActorType.Device ? requester.Id : null,
            TargetDeviceId = targetDeviceId,
            Type = type,
            Payload = payloadJson,
            Status = JarvisTaskStatus.Pending,
            CorrelationId = correlationId,
            IdempotencyKey = idempotencyKey,
            RequestHash = requestHash,
            MaxAttempts = maxAttempts,
            ExpiresAt = effectiveExpiresAt,
            MaxDurationSeconds = maxDurationSeconds,
            CreatedAt = now,
            SignatureAlg = signature?.Alg,
            SignatureKeyId = signature?.KeyId,
            SignatureValue = signature?.Value,
        };
        task.RaiseTaskEvent(
            EventTypes.TaskCreated,
            now,
            requester,
            new()
            {
                ["type"] = type,
                ["targetDeviceId"] = targetDeviceId,
                ["channel"] = channel,
                ["expiresAt"] = task.ExpiresAt,
                ["signed"] = signature is not null,
                ["signatureKeyId"] = signature?.KeyId,
            });
        return task;
    }

    // ---- Transiciones del sistema ------------------------------------------------

    /// <summary>T2: Pending → Dispatched.</summary>
    public void Dispatch(DateTimeOffset now)
    {
        EnsureStatus(JarvisTaskStatus.Pending);
        AttemptCount++;
        DispatchedAt = now;
        Status = JarvisTaskStatus.Dispatched;
        StatusReason = null;
        RaiseTaskEvent(EventTypes.TaskDispatched, now, Actor.System, new() { ["attempt"] = AttemptCount });
    }

    /// <summary>T5 / T6: sin ACK. Vuelve a Pending o falla si se agotaron los intentos.</summary>
    public bool HandleAckTimeout(DateTimeOffset now)
    {
        if (Status != JarvisTaskStatus.Dispatched)
        {
            return false;
        }

        if (AttemptCount >= MaxAttempts)
        {
            Terminate(JarvisTaskStatus.Failed, TaskErrorCodes.DeliveryFailed, "No acknowledgement after max attempts.", now, Actor.System);
            return true;
        }

        Status = JarvisTaskStatus.Pending;
        StatusReason = "ack_timeout";
        return true;
    }

    /// <summary>Reconciliación en presence.hello cuando la tarea no está en openTaskIds (T5 / T15).</summary>
    public bool HandleMissingOnDevice(DateTimeOffset now)
    {
        switch (Status)
        {
            case JarvisTaskStatus.Dispatched:
                Status = JarvisTaskStatus.Pending;
                StatusReason = "not_on_device";
                return true;
            case JarvisTaskStatus.Delivered or JarvisTaskStatus.Running or JarvisTaskStatus.WaitingForApproval:
                Terminate(JarvisTaskStatus.Failed, TaskErrorCodes.DeviceLost, "Device reconnected without the task.", now, Actor.System);
                return true;
            default:
                return false;
        }
    }

    /// <summary>T4 / T8: expiración de tareas no entregadas.</summary>
    public bool ExpireIfDue(DateTimeOffset now)
    {
        var due = Status switch
        {
            JarvisTaskStatus.Pending => now > ExpiresAt,
            JarvisTaskStatus.Dispatched => now > ExpiresAt + DispatchedExpiryGrace,
            _ => false,
        };
        if (!due)
        {
            return false;
        }

        var lastStatus = Status;
        Status = JarvisTaskStatus.Expired;
        CompletedAt = now;
        ErrorCode = TaskErrorCodes.NotDelivered;
        ErrorMessage = "Not delivered before expiration.";
        RaiseTaskEvent(EventTypes.TaskExpired, now, Actor.System, new() { ["lastStatus"] = lastStatus.ToString() });
        return true;
    }

    public DateTimeOffset? ExecutionDeadline =>
        DeliveredAt is { } delivered
            ? delivered + (MaxDurationSeconds is { } s ? TimeSpan.FromSeconds(s) : SystemMaxDuration)
            : null;

    /// <summary>T16: excedió la duración máxima.</summary>
    public bool TimeoutIfDue(DateTimeOffset now)
    {
        if (Status is not (JarvisTaskStatus.Delivered or JarvisTaskStatus.Running or JarvisTaskStatus.WaitingForApproval)
            || ExecutionDeadline is not { } deadline || now <= deadline)
        {
            return false;
        }

        Terminate(JarvisTaskStatus.Failed, TaskErrorCodes.ExecutionTimeout, "Exceeded maximum duration.", now, Actor.System);
        return true;
    }

    /// <summary>El target fue revocado.</summary>
    public bool CancelByRevocation(DateTimeOffset now)
    {
        if (IsTerminal)
        {
            return false;
        }

        Terminate(JarvisTaskStatus.Cancelled, TaskErrorCodes.DeviceRevoked, "Target device revoked.", now, null);
        return true;
    }

    /// <summary>T3 / T17.</summary>
    public CancelOutcome RequestCancel(Actor actor, DateTimeOffset now)
    {
        if (IsTerminal)
        {
            throw new DomainException(DomainErrorCodes.InvalidStateTransition, $"Task is already {Status}.");
        }

        if (Status == JarvisTaskStatus.Pending)
        {
            CancelRequestedAt ??= now;
            Terminate(JarvisTaskStatus.Cancelled, TaskErrorCodes.UserRequest, "Cancelled before delivery.", now, actor);
            return CancelOutcome.CancelledImmediately;
        }

        if (CancelRequestedAt is not null)
        {
            return CancelOutcome.AlreadyRequested;
        }

        CancelRequestedAt = now;
        RaiseTaskEvent(EventTypes.TaskCancelRequested, now, actor, []);
        return CancelOutcome.CancelRequested;
    }

    // ---- Mensajes del device (up/tasks) -----------------------------------------

    public TaskUpdateOutcome Acknowledge(DateTimeOffset now) => ApplyDeviceTransition(JarvisTaskStatus.Delivered, now, null);

    public TaskUpdateOutcome Reject(TaskError error, DateTimeOffset now) =>
        ApplyDeviceTransition(JarvisTaskStatus.Failed, now, error);

    public TaskUpdateOutcome Start(DateTimeOffset now) => ApplyDeviceTransition(JarvisTaskStatus.Running, now, null);

    public TaskUpdateOutcome RequireApproval(string? reason, DateTimeOffset now) =>
        ApplyDeviceTransition(JarvisTaskStatus.WaitingForApproval, now, null, reason);

    public TaskUpdateOutcome Complete(string resultJson, DateTimeOffset now)
    {
        if (DomainRules.Utf8Length(resultJson) > DomainRules.MaxResultBytes)
        {
            return ApplyDeviceTransition(JarvisTaskStatus.Failed, now, new TaskError(TaskErrorCodes.ResultTooLarge, "Result exceeds 32 KiB."));
        }

        return ApplyDeviceTransition(JarvisTaskStatus.Completed, now, null, result: resultJson);
    }

    public TaskUpdateOutcome Fail(TaskError error, DateTimeOffset now) => ApplyDeviceTransition(JarvisTaskStatus.Failed, now, error);

    public TaskUpdateOutcome ConfirmCancelled(string reason, DateTimeOffset now) =>
        ApplyDeviceTransition(JarvisTaskStatus.Cancelled, now, new TaskError(reason, null));

    public TaskUpdateOutcome ReportProgress(short? percent, string? message, DateTimeOffset now)
    {
        var check = CheckDeviceMessage();
        if (check is not null)
        {
            return check.Value;
        }

        if (percent is < 0 or > 100)
        {
            percent = null;
        }

        ProgressPercent = percent;
        ProgressMessage = message is null ? null : DomainRules.Truncate(message, DomainRules.MaxProgressMessageLength);
        LastUpdateAt = now;
        return TaskUpdateOutcome.Applied;
    }

    private TaskUpdateOutcome? CheckDeviceMessage()
    {
        if (IsTerminal)
        {
            return TaskUpdateOutcome.Terminal;
        }

        if (Status == JarvisTaskStatus.Pending && AttemptCount == 0)
        {
            return TaskUpdateOutcome.SecurityViolation;
        }

        return null;
    }

    private TaskUpdateOutcome ApplyDeviceTransition(
        JarvisTaskStatus target,
        DateTimeOffset now,
        TaskError? error,
        string? reason = null,
        string? result = null)
    {
        var check = CheckDeviceMessage();
        if (check is not null)
        {
            return check.Value;
        }

        // Una tarea revertida a Pending (attempt_count > 0) que recibe mensajes se trata como Dispatched.
        var current = Status == JarvisTaskStatus.Pending ? JarvisTaskStatus.Dispatched : Status;
        var currentRank = Rank(current);
        var targetRank = Rank(target);
        LastUpdateAt = now;

        if (targetRank < currentRank)
        {
            return TaskUpdateOutcome.Stale;
        }

        if (targetRank == currentRank && current == target)
        {
            return TaskUpdateOutcome.Duplicate;
        }

        var device = Actor.Device(TargetDeviceId);

        // Cualquier mensaje posterior al ack prueba la entrega.
        if (currentRank <= 1)
        {
            DeliveredAt ??= now;
            RaiseTaskEvent(EventTypes.TaskDelivered, now, device, []);
        }

        var previous = current;
        switch (target)
        {
            case JarvisTaskStatus.Delivered:
                Status = JarvisTaskStatus.Delivered;
                break;
            case JarvisTaskStatus.Running:
                Status = JarvisTaskStatus.Running;
                if (previous == JarvisTaskStatus.WaitingForApproval)
                {
                    // "device": es lo único que se sabe aquí. Si la decisión vino firmada desde el
                    // Cloud, el origen está en el APPROVAL_DECISION_SENT del mismo taskId.
                    RaiseTaskEvent(EventTypes.ApprovalGranted, now, device, new() { ["decidedBy"] = "device" });
                }

                if (StartedAt is null)
                {
                    StartedAt = now;
                    RaiseTaskEvent(EventTypes.TaskStarted, now, device, []);
                }

                break;
            case JarvisTaskStatus.WaitingForApproval:
                Status = JarvisTaskStatus.WaitingForApproval;
                RaiseTaskEvent(
                    EventTypes.ApprovalRequested,
                    now,
                    device,
                    new() { ["reason"] = DomainRules.Truncate(reason, DomainRules.MaxProgressMessageLength) });
                break;
            case JarvisTaskStatus.Completed:
                Status = JarvisTaskStatus.Completed;
                Result = result ?? "{}";
                CompletedAt = now;
                RaiseTaskEvent(EventTypes.TaskCompleted, now, device, new() { ["resultSizeBytes"] = DomainRules.Utf8Length(Result) });
                break;
            case JarvisTaskStatus.Failed:
                Terminate(JarvisTaskStatus.Failed, error!.Code, error.Message, now, device);
                break;
            case JarvisTaskStatus.Cancelled:
                if (error!.Code == TaskErrorCodes.ApprovalDenied)
                {
                    RaiseTaskEvent(EventTypes.ApprovalDenied, now, device, new() { ["decidedBy"] = "local" });
                }

                Terminate(JarvisTaskStatus.Cancelled, error.Code, error.Message, now, device);
                break;
            default:
                throw new DomainException(DomainErrorCodes.InvalidStateTransition, $"Unsupported target status {target}.");
        }

        StatusReason = null;
        return TaskUpdateOutcome.Applied;
    }

    private void Terminate(JarvisTaskStatus terminal, string code, string? message, DateTimeOffset now, Actor? actor)
    {
        Status = terminal;
        CompletedAt = now;
        ErrorCode = DomainRules.Truncate(string.IsNullOrWhiteSpace(code) ? TaskErrorCodes.ExecutionError : code, DomainRules.MaxErrorCodeLength);
        ErrorMessage = message is null ? null : DomainRules.Truncate(message, DomainRules.MaxErrorMessageLength);
        if (terminal == JarvisTaskStatus.Cancelled)
        {
            RaiseTaskEvent(EventTypes.TaskCancelled, now, actor, new() { ["reason"] = ErrorCode });
        }
        else
        {
            RaiseTaskEvent(EventTypes.TaskFailed, now, actor, new() { ["code"] = ErrorCode });
        }
    }

    private void EnsureStatus(JarvisTaskStatus expected)
    {
        if (Status != expected)
        {
            throw new DomainException(DomainErrorCodes.InvalidStateTransition, $"Expected {expected} but task is {Status}.");
        }
    }

    private void RaiseTaskEvent(string type, DateTimeOffset now, Actor? actor, Dictionary<string, object?> data) =>
        Raise(new DomainEvent(type, OwnerId, now, DeviceId: TargetDeviceId, TaskId: Id, Data: data, Actor: actor, CorrelationId: CorrelationId));
}
