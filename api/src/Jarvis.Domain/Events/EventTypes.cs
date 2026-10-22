namespace Jarvis.Domain.Events;

/// <summary>Catálogo de tipos de evento (docs/02-domain/event-model.md).</summary>
public static class EventTypes
{
    public const string DeviceRegistered = "DEVICE_REGISTERED";
    public const string DeviceUpdated = "DEVICE_UPDATED";
    public const string DeviceDisabled = "DEVICE_DISABLED";
    public const string DeviceEnabled = "DEVICE_ENABLED";
    public const string DeviceRevoked = "DEVICE_REVOKED";
    public const string DeviceCredentialRotated = "DEVICE_CREDENTIAL_ROTATED";
    public const string DeviceConnected = "DEVICE_CONNECTED";
    public const string DeviceDisconnected = "DEVICE_DISCONNECTED";
    public const string DeviceAuthFailed = "DEVICE_AUTH_FAILED";

    public const string TaskCreated = "TASK_CREATED";
    public const string TaskRejected = "TASK_REJECTED";
    public const string TaskDispatched = "TASK_DISPATCHED";
    public const string TaskDelivered = "TASK_DELIVERED";
    public const string TaskStarted = "TASK_STARTED";
    public const string TaskCompleted = "TASK_COMPLETED";
    public const string TaskFailed = "TASK_FAILED";
    public const string TaskCancelRequested = "TASK_CANCEL_REQUESTED";
    public const string TaskCancelled = "TASK_CANCELLED";
    public const string TaskExpired = "TASK_EXPIRED";

    public const string ApprovalRequested = "APPROVAL_REQUESTED";
    /// <summary>El owner firmó una decisión y el Cloud la transmitió. No implica que el device la aplicara.</summary>
    public const string ApprovalDecisionSent = "APPROVAL_DECISION_SENT";
    public const string ApprovalGranted = "APPROVAL_GRANTED";
    public const string ApprovalDenied = "APPROVAL_DENIED";

    public const string UserCreated = "USER_CREATED";
    public const string UserLoginSucceeded = "USER_LOGIN_SUCCEEDED";
    public const string UserLoginFailed = "USER_LOGIN_FAILED";
    public const string UserLockedOut = "USER_LOCKED_OUT";
    public const string GrantCreated = "GRANT_CREATED";
    public const string GrantDeleted = "GRANT_DELETED";
    public const string SecurityViolation = "SECURITY_VIOLATION";

    // Memoria, sincronización y jobs: solo metadatos, nunca contenido.
    public const string SyncScopesChanged = "SYNC_SCOPES_CHANGED";
    public const string SyncPushCompleted = "SYNC_PUSH_COMPLETED";
    public const string SyncConflictDetected = "SYNC_CONFLICT_DETECTED";
    public const string SyncConflictResolved = "SYNC_CONFLICT_RESOLVED";
    public const string SkillStatusChanged = "SKILL_STATUS_CHANGED";
    public const string SkillProposed = "SKILL_PROPOSED";
    public const string JobCreated = "JOB_CREATED";
    public const string JobApproved = "JOB_APPROVED";
    public const string JobRunStarted = "JOB_RUN_STARTED";
    public const string JobRunSucceeded = "JOB_RUN_SUCCEEDED";
    public const string JobRunFailed = "JOB_RUN_FAILED";
}

/// <summary>Fila del event log operacional (append-only).</summary>
public sealed class EventRecord
{
    public long Seq { get; set; }

    public Guid Id { get; set; }

    public Guid? OwnerId { get; set; }

    public string Type { get; set; } = string.Empty;

    public DateTimeOffset OccurredAt { get; set; }

    public DateTimeOffset RecordedAt { get; set; }

    public Guid CorrelationId { get; set; }

    public Common.ActorType ActorType { get; set; }

    public Guid? ActorId { get; set; }

    public Guid? DeviceId { get; set; }

    public Guid? TaskId { get; set; }

    /// <summary>JSON (jsonb) pequeño y sin datos sensibles.</summary>
    public string Data { get; set; } = "{}";
}
