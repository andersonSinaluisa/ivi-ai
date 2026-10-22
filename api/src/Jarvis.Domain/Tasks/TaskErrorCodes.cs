namespace Jarvis.Domain.Tasks;

/// <summary>Valores de <c>error_code</c> de tareas (docs/03-protocols/error-model.md).</summary>
public static class TaskErrorCodes
{
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
    public const string DeviceRevoked = "DEVICE_REVOKED";
    public const string NotDelivered = "NOT_DELIVERED";
}
