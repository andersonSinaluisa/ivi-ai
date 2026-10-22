namespace Jarvis.Contracts;

/// <summary>Catálogo único de códigos de error REST/MQTT (docs/03-protocols/error-model.md).</summary>
public static class ErrorCodes
{
    public const string ValidationFailed = "VALIDATION_FAILED";
    public const string Unauthenticated = "UNAUTHENTICATED";
    public const string InvalidCredentials = "INVALID_CREDENTIALS";
    public const string InvalidRefreshToken = "INVALID_REFRESH_TOKEN";
    public const string Forbidden = "FORBIDDEN";
    public const string TaskNotGranted = "TASK_NOT_GRANTED";
    public const string DeviceDisabled = "DEVICE_DISABLED";
    public const string NotFound = "NOT_FOUND";
    public const string DeviceNameTaken = "DEVICE_NAME_TAKEN";
    public const string DeviceRevoked = "DEVICE_REVOKED";
    public const string GrantAlreadyExists = "GRANT_ALREADY_EXISTS";
    public const string TargetDeviceUnavailable = "TARGET_DEVICE_UNAVAILABLE";
    public const string InvalidStateTransition = "INVALID_STATE_TRANSITION";
    public const string PayloadTooLarge = "PAYLOAD_TOO_LARGE";
    public const string IdempotencyKeyReused = "IDEMPOTENCY_KEY_REUSED";
    public const string IdempotencyKeyRequired = "IDEMPOTENCY_KEY_REQUIRED";
    public const string TaskIdInUse = "TASK_ID_IN_USE";
    public const string ApprovalAlreadyDecided = "APPROVAL_ALREADY_DECIDED";
    public const string NonceReused = "NONCE_REUSED";
    public const string TaskTypeNotSupported = "TASK_TYPE_NOT_SUPPORTED";
    public const string SameDevice = "SAME_DEVICE";
    public const string DeviceLimitReached = "DEVICE_LIMIT_REACHED";
    public const string AccountLocked = "ACCOUNT_LOCKED";
    public const string RateLimited = "RATE_LIMITED";
    public const string StaleMessage = "STALE_MESSAGE";
    public const string InternalError = "INTERNAL_ERROR";
    public const string ServiceUnavailable = "SERVICE_UNAVAILABLE";

    // Memoria, sincronización y jobs (docs/03-protocols/sync-protocol.md)
    public const string SyncScopeMissing = "SYNC_SCOPE_MISSING";
    public const string UnsupportedProtocol = "UNSUPPORTED_PROTOCOL";
    public const string ContentRejected = "CONTENT_REJECTED";
    public const string ReferenceNotFound = "REFERENCE_NOT_FOUND";
    public const string StaleEmbedding = "STALE_EMBEDDING";
    public const string ConflictAlreadyResolved = "CONFLICT_ALREADY_RESOLVED";
    public const string JobTypeUnknown = "JOB_TYPE_UNKNOWN";
    public const string JobNotRunnable = "JOB_NOT_RUNNABLE";

    // error_code de tareas
    public const string ExecutionError = "EXECUTION_ERROR";
    public const string NotPermitted = "NOT_PERMITTED";
    public const string UnsupportedTaskType = "UNSUPPORTED_TASK_TYPE";
    public const string InvalidPayload = "INVALID_PAYLOAD";
    public const string ExpiredOnArrival = "EXPIRED_ON_ARRIVAL";
    public const string DeliveryFailed = "DELIVERY_FAILED";
    public const string ExecutionTimeout = "EXECUTION_TIMEOUT";
    public const string DeviceLost = "DEVICE_LOST";
    public const string ResultTooLarge = "RESULT_TOO_LARGE";
    public const string UserRequest = "USER_REQUEST";
    public const string ApprovalDenied = "APPROVAL_DENIED";
    public const string LocalPolicy = "LOCAL_POLICY";
    public const string NotDelivered = "NOT_DELIVERED";
}
