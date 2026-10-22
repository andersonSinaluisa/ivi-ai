using System.Text.Json;

namespace Jarvis.Contracts.Mqtt;

public sealed record ErrorInfo(string Code, string? Message = null, bool? Retryable = null);

public sealed record RequestedByInfo(string Type, Guid Id);

/// <summary>Firma Ed25519 del originador (ADR-012). El Cloud la transporta; no la produce ni la verifica.</summary>
public sealed record SignatureInfo(string Alg, string KeyId, string Value);

public sealed record ProgressInfo(int? Percent, string? Message);

// ---- up/requests -------------------------------------------------------------

public sealed record TaskCreateData(
    string? IdempotencyKey,
    Guid? TargetDeviceId,
    string? Type,
    JsonElement? Payload,
    int? ExpiresInSeconds,
    int? MaxDurationSeconds);

public sealed record TaskGetData(Guid TaskId);

public sealed record TaskCancelRequestData(Guid TaskId);

// ---- down/notifications ------------------------------------------------------

public sealed record RequestAcceptedData(string RequestType, Guid TaskId, string Status, bool Duplicate);

public sealed record RequestRejectedData(string RequestType, ErrorInfo Error);

public sealed record TaskUpdateData(
    Guid TaskId,
    string Type,
    Guid TargetDeviceId,
    string Status,
    DateTimeOffset UpdatedAt,
    ProgressInfo? Progress,
    JsonElement? Result,
    ErrorInfo? Error);

// ---- down/tasks --------------------------------------------------------------

public sealed record TaskDispatchData(
    Guid TaskId,
    string Type,
    JsonElement Payload,
    int Attempt,
    DateTimeOffset CreatedAt,
    DateTimeOffset ExpiresAt,
    int? MaxDurationSeconds,
    RequestedByInfo RequestedBy,
    bool CancelRequested,
    SignatureInfo? Signature);

public sealed record TaskCancelData(Guid TaskId, string Reason);

/// <summary>HU-057: decisión de aprobación firmada por el cliente del owner; el Cloud solo la retransmite.</summary>
public sealed record TaskApprovalDecisionData(
    Guid TaskId,
    string Decision,
    RequestedByInfo DecidedBy,
    DateTimeOffset DecidedAt,
    string Nonce,
    SignatureInfo Signature);

public sealed record TaskUpdateAckData(Guid TaskId, string Status);

// ---- up/tasks ----------------------------------------------------------------

public sealed record TaskRefData(Guid TaskId);

public sealed record TaskRejectedData(Guid TaskId, ErrorInfo Error);

public sealed record TaskStartedData(Guid TaskId, DateTimeOffset? StartedAt);

public sealed record TaskProgressData(Guid TaskId, int? Percent, string? Message);

public sealed record TaskApprovalRequiredData(Guid TaskId, string? Reason);

public sealed record TaskCompletedData(Guid TaskId, DateTimeOffset? CompletedAt, JsonElement? Result);

public sealed record TaskFailedData(Guid TaskId, ErrorInfo Error);

public sealed record TaskCancelledData(Guid TaskId, string? Reason);

// ---- presence ----------------------------------------------------------------

public sealed record PresenceHelloData(
    string SessionId,
    string? ProtocolVersion,
    string? SoftwareVersion,
    string? Platform,
    IReadOnlyList<string>? Capabilities,
    IReadOnlyList<string>? SupportedTaskTypes,
    IReadOnlyList<Guid>? OpenTaskIds,
    bool Busy);

public sealed record PresenceHelloAckData(string SessionId, DateTimeOffset ServerTime, int HeartbeatIntervalSeconds, int MaxMessageBytes);

public sealed record PresenceHeartbeatData(string SessionId, bool Busy, int? OpenTaskCount);

public sealed record PresenceGoodbyeData(string SessionId, string? Reason);

public sealed record PresenceOfflineData(string SessionId);
