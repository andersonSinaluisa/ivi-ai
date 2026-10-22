using System.Text.Json;

namespace Jarvis.Contracts.Rest;

// ---- Auth --------------------------------------------------------------------

public sealed record LoginRequest(string? Email, string? Password);

public sealed record RefreshRequest(string? RefreshToken);

public sealed record LogoutRequest(string? RefreshToken);

public sealed record DeviceTokenRequest(Guid? DeviceId, string? Secret);

public sealed record UserSummaryDto(Guid Id, string Email, string DisplayName);

public sealed record LoginResponse(
    string AccessToken,
    DateTimeOffset AccessTokenExpiresAt,
    string RefreshToken,
    DateTimeOffset RefreshTokenExpiresAt,
    UserSummaryDto User);

public sealed record AccessTokenResponse(string AccessToken, DateTimeOffset AccessTokenExpiresAt);

public sealed record MeResponse(
    string PrincipalType,
    Guid Id,
    Guid OwnerId,
    string? Email = null,
    string? DisplayName = null,
    string? Name = null,
    string? Type = null);

// ---- Devices -----------------------------------------------------------------

public sealed record CreateDeviceRequest(string? Name, string? Type);

public sealed record RenameDeviceRequest(string? Name);

public sealed record RevokeDeviceRequest(string? Reason);

public sealed record CredentialSummaryDto(Guid Id, string? Hint, DateTimeOffset CreatedAt, DateTimeOffset? LastUsedAt);

public sealed record DeviceDto(
    Guid Id,
    string Name,
    string Type,
    string Status,
    string AdminState,
    string PresenceState,
    bool Busy,
    IReadOnlyList<string> Capabilities,
    IReadOnlyList<string> SupportedTaskTypes,
    JsonElement ReportedInfo,
    DateTimeOffset? LastSeenAt,
    DateTimeOffset? LastConnectedAt,
    DateTimeOffset? LastDisconnectedAt,
    bool BrokerSynced,
    CredentialSummaryDto? Credential,
    DateTimeOffset CreatedAt,
    DateTimeOffset UpdatedAt);

public sealed record MqttConnectionInfo(string Host, int Port, string Username, string ClientId);

public sealed record IssuedCredentialDto(Guid DeviceId, string Secret, MqttConnectionInfo Mqtt, string Warning);

public sealed record RegisterDeviceResponse(DeviceDto Device, IssuedCredentialDto Credential);

public sealed record CreateGrantRequest(Guid? TargetDeviceId, string? TaskType);

public sealed record GrantDto(Guid Id, Guid SourceDeviceId, Guid TargetDeviceId, string TaskType, DateTimeOffset CreatedAt);

// ---- Tasks -------------------------------------------------------------------

public sealed record CreateTaskRequest(
    Guid? TargetDeviceId,
    string? Type,
    JsonElement? Payload,
    int? ExpiresInSeconds,
    int? MaxDurationSeconds,
    Guid? CorrelationId,
    /// <summary>Solo en tareas firmadas: el id que el cliente firmó (ADR-012).</summary>
    Guid? TaskId = null,
    /// <summary>Expiración absoluta; obligatoria al firmar, excluyente con <see cref="ExpiresInSeconds"/>.</summary>
    DateTimeOffset? ExpiresAt = null,
    SignatureDto? Signature = null);

public sealed record CancelTaskRequest(string? Reason);

/// <summary>Firma Ed25519 del cliente. El Cloud la transporta; el device la verifica (ADR-012).</summary>
public sealed record SignatureDto(string? Alg, string? KeyId, string? Value);

/// <summary>HU-057: decisión de aprobación firmada por el cliente del owner.</summary>
public sealed record ApprovalDecisionRequest(
    string? Decision,
    string? Nonce,
    DateTimeOffset? DecidedAt,
    SignatureDto? Signature);

public sealed record TaskApprovalDto(
    Guid Id,
    Guid TaskId,
    string Decision,
    RequestedByDto DecidedBy,
    DateTimeOffset DecidedAt,
    string Nonce,
    SignatureDto Signature,
    DateTimeOffset CreatedAt);

public sealed record RequestedByDto(string Type, Guid Id);

public sealed record TaskErrorDto(string Code, string? Message);

public sealed record TaskProgressDto(int? Percent, string? Message);

public sealed record TaskDto(
    Guid Id,
    string Type,
    string Status,
    JsonElement? Payload,
    RequestedByDto RequestedBy,
    Guid? SourceDeviceId,
    Guid TargetDeviceId,
    Guid CorrelationId,
    string? IdempotencyKey,
    int AttemptCount,
    DateTimeOffset ExpiresAt,
    int? MaxDurationSeconds,
    DateTimeOffset CreatedAt,
    DateTimeOffset? DispatchedAt,
    DateTimeOffset? DeliveredAt,
    DateTimeOffset? StartedAt,
    DateTimeOffset? CompletedAt,
    DateTimeOffset? CancelRequestedAt,
    TaskProgressDto? Progress,
    JsonElement? Result,
    TaskErrorDto? Error,
    SignatureDto? Signature = null);

// ---- Events ------------------------------------------------------------------

public sealed record ActorDto(string Type, Guid? Id);

public sealed record EventDto(
    long Seq,
    Guid Id,
    string Type,
    DateTimeOffset OccurredAt,
    DateTimeOffset RecordedAt,
    Guid CorrelationId,
    ActorDto Actor,
    Guid? DeviceId,
    Guid? TaskId,
    JsonElement Data);

public sealed record EventsPage(IReadOnlyList<EventDto> Items, long? LastSeq, bool HasMore);

// ---- Common ------------------------------------------------------------------

public sealed record Page<T>(IReadOnlyList<T> Items, string? NextCursor);

/// <summary>Notificación enviada por el hub SignalR (método de cliente <c>event</c>).</summary>
public sealed record RealtimeNotification(
    long? Seq,
    string Name,
    DateTimeOffset OccurredAt,
    Guid? CorrelationId,
    Guid? DeviceId,
    Guid? TaskId,
    JsonElement Data);
