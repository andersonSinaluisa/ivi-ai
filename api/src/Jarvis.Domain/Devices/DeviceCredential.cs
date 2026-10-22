using Jarvis.Domain.Common;
using Jarvis.Domain.Events;

namespace Jarvis.Domain.Devices;

public enum CredentialKind
{
    Secret,
    ClientCertificate,
}

public enum CredentialStatus
{
    Active,
    Revoked,
}

public sealed class DeviceCredential
{
    private DeviceCredential()
    {
    }

    public Guid Id { get; private set; }

    public Guid DeviceId { get; private set; }

    public CredentialKind Kind { get; private set; }

    public byte[]? SecretHash { get; private set; }

    public string? SecretHint { get; private set; }

    public string? CertificateThumbprint { get; private set; }

    public CredentialStatus Status { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public DateTimeOffset? ExpiresAt { get; private set; }

    public DateTimeOffset? RevokedAt { get; private set; }

    public string? RevokedReason { get; private set; }

    public DateTimeOffset? LastUsedAt { get; private set; }

    public static DeviceCredential CreateSecret(Guid id, Guid deviceId, byte[] secretHash, string secretHint, DateTimeOffset now)
    {
        if (secretHash.Length != 32)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Secret hash must be 32 bytes.");
        }

        return new DeviceCredential
        {
            Id = id,
            DeviceId = deviceId,
            Kind = CredentialKind.Secret,
            SecretHash = secretHash,
            SecretHint = secretHint,
            Status = CredentialStatus.Active,
            CreatedAt = now,
        };
    }

    public bool IsUsable(DateTimeOffset now) => Status == CredentialStatus.Active && (ExpiresAt is null || ExpiresAt > now);

    public void Revoke(string reason, DateTimeOffset now)
    {
        if (Status == CredentialStatus.Revoked)
        {
            return;
        }

        Status = CredentialStatus.Revoked;
        RevokedAt = now;
        RevokedReason = DomainRules.Truncate(reason, 200);
    }

    /// <summary>Actualiza <see cref="LastUsedAt"/> como máximo una vez por minuto.</summary>
    public bool MarkUsed(DateTimeOffset now)
    {
        if (LastUsedAt is { } last && now - last < TimeSpan.FromMinutes(1))
        {
            return false;
        }

        LastUsedAt = now;
        return true;
    }
}

/// <summary>Permiso explícito: el device origen puede crear tareas <see cref="TaskType"/> para el device destino.</summary>
public sealed class DeviceTaskGrant : Entity
{
    private DeviceTaskGrant()
    {
    }

    public Guid OwnerId { get; private set; }

    public Guid SourceDeviceId { get; private set; }

    public Guid TargetDeviceId { get; private set; }

    public string TaskType { get; private set; } = string.Empty;

    public Guid CreatedByUserId { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public static DeviceTaskGrant Create(Guid id, Device source, Device target, string taskType, Guid createdByUserId, DateTimeOffset now)
    {
        if (!DomainRules.IsValidTaskType(taskType))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Invalid task type.");
        }

        if (source.Id == target.Id)
        {
            throw new DomainException("SAME_DEVICE", "Source and target must be different devices.");
        }

        if (source.OwnerId != target.OwnerId)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Devices belong to different owners.");
        }

        if (source.AdminState == DeviceAdminState.Revoked || target.AdminState == DeviceAdminState.Revoked)
        {
            throw new DomainException(DomainErrorCodes.DeviceRevoked, "Cannot grant on revoked devices.");
        }

        var grant = new DeviceTaskGrant
        {
            Id = id,
            OwnerId = source.OwnerId,
            SourceDeviceId = source.Id,
            TargetDeviceId = target.Id,
            TaskType = taskType,
            CreatedByUserId = createdByUserId,
            CreatedAt = now,
        };
        grant.Raise(grant.CreateEvent(EventTypes.GrantCreated, now));
        return grant;
    }

    /// <summary>Registra el evento antes de eliminar la fila.</summary>
    public void MarkDeleted(DateTimeOffset now) => Raise(CreateEvent(EventTypes.GrantDeleted, now));

    private DomainEvent CreateEvent(string type, DateTimeOffset now) =>
        new(
            type,
            OwnerId,
            now,
            DeviceId: SourceDeviceId,
            Data: new Dictionary<string, object?>
            {
                ["sourceDeviceId"] = SourceDeviceId,
                ["targetDeviceId"] = TargetDeviceId,
                ["taskType"] = TaskType,
            });
}
