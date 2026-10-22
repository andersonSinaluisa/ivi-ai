using Jarvis.Domain.Common;
using Jarvis.Domain.Events;

namespace Jarvis.Domain.Devices;

public enum DeviceType
{
    Windows,
    ESP32,
    Mobile,
    Web,
    Unknown,
}

public enum DeviceAdminState
{
    Active,
    Disabled,
    Revoked,
}

public enum PresenceState
{
    Online,
    Stale,
    Offline,
}

/// <summary>Estado calculado expuesto por la API (docs/02-domain/device-model.md).</summary>
public enum DeviceStatus
{
    Online,
    Busy,
    Stale,
    Offline,
    Disabled,
}

/// <summary>Resultado de actualizar presencia por un mensaje entrante.</summary>
public enum PresenceChange
{
    None,
    RecoveredFromStale,
    RecoveredFromOffline,
}

public sealed class Device : Entity
{
    private readonly List<DeviceCredential> _credentials = [];

    private Device()
    {
    }

    public Guid OwnerId { get; private set; }

    public string Name { get; private set; } = string.Empty;

    public DeviceType Type { get; private set; }

    public DeviceAdminState AdminState { get; private set; }

    public PresenceState PresenceState { get; private set; }

    public bool ReportedBusy { get; private set; }

    public string? CurrentSessionId { get; private set; }

    public List<string> Capabilities { get; private set; } = [];

    public List<string> SupportedTaskTypes { get; private set; } = [];

    /// <summary>JSON (jsonb) reportado por el device en <c>presence.hello</c>.</summary>
    public string ReportedInfo { get; private set; } = "{}";

    public DateTimeOffset? LastSeenAt { get; private set; }

    public DateTimeOffset? LastConnectedAt { get; private set; }

    public DateTimeOffset? LastDisconnectedAt { get; private set; }

    public DateTimeOffset? BrokerSyncedAt { get; private set; }

    public DateTimeOffset? DisabledAt { get; private set; }

    public DateTimeOffset? RevokedAt { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public DateTimeOffset UpdatedAt { get; private set; }

    public IReadOnlyList<DeviceCredential> Credentials => _credentials;

    public DeviceCredential? ActiveCredential => _credentials.FirstOrDefault(c => c.Status == CredentialStatus.Active);

    public DeviceStatus Status => ComputeStatus(AdminState, PresenceState, ReportedBusy);

    public bool IsActive => AdminState == DeviceAdminState.Active;

    public static DeviceStatus ComputeStatus(DeviceAdminState admin, PresenceState presence, bool busy) =>
        admin != DeviceAdminState.Active ? DeviceStatus.Disabled
        : presence == PresenceState.Offline ? DeviceStatus.Offline
        : presence == PresenceState.Stale ? DeviceStatus.Stale
        : busy ? DeviceStatus.Busy
        : DeviceStatus.Online;

    public static Device Register(Guid id, Guid ownerId, string name, DeviceType type, DeviceCredential credential, DateTimeOffset now)
    {
        EnsureValidName(name);
        var device = new Device
        {
            Id = id,
            OwnerId = ownerId,
            Name = name,
            Type = type,
            AdminState = DeviceAdminState.Active,
            PresenceState = PresenceState.Offline,
            CreatedAt = now,
            UpdatedAt = now,
        };
        device._credentials.Add(credential);
        device.RaiseDeviceEvent(EventTypes.DeviceRegistered, now, new() { ["name"] = name, ["type"] = type.ToString() });
        return device;
    }

    public bool Rename(string name, DateTimeOffset now)
    {
        EnsureNotRevoked();
        EnsureValidName(name);
        if (Name == name)
        {
            return false;
        }

        Name = name;
        UpdatedAt = now;
        RaiseDeviceEvent(EventTypes.DeviceUpdated, now, new() { ["changes"] = new[] { "name" } });
        return true;
    }

    /// <summary>Deshabilita el device. Devuelve false si ya estaba deshabilitado (idempotente, sin eventos).</summary>
    public bool Disable(DateTimeOffset now)
    {
        EnsureNotRevoked();
        if (AdminState == DeviceAdminState.Disabled)
        {
            return false;
        }

        AdminState = DeviceAdminState.Disabled;
        DisabledAt = now;
        UpdatedAt = now;
        RaiseDeviceEvent(EventTypes.DeviceDisabled, now, []);
        ForceOffline("disabled", now);
        return true;
    }

    public bool Enable(DateTimeOffset now)
    {
        EnsureNotRevoked();
        if (AdminState == DeviceAdminState.Active)
        {
            return false;
        }

        AdminState = DeviceAdminState.Active;
        DisabledAt = null;
        UpdatedAt = now;
        RaiseDeviceEvent(EventTypes.DeviceEnabled, now, []);
        return true;
    }

    /// <summary>Revocación irreversible. Devuelve false si ya estaba revocado.</summary>
    public bool Revoke(string? reason, int cancelledTasks, DateTimeOffset now)
    {
        if (AdminState == DeviceAdminState.Revoked)
        {
            return false;
        }

        AdminState = DeviceAdminState.Revoked;
        RevokedAt = now;
        UpdatedAt = now;
        foreach (var credential in _credentials)
        {
            credential.Revoke(reason ?? "device revoked", now);
        }

        RaiseDeviceEvent(EventTypes.DeviceRevoked, now, new() { ["cancelledTasks"] = cancelledTasks });
        ForceOffline("disabled", now);
        return true;
    }

    public void RotateCredential(DeviceCredential newCredential, DateTimeOffset now)
    {
        EnsureNotRevoked();
        foreach (var credential in _credentials.Where(c => c.Status == CredentialStatus.Active))
        {
            credential.Revoke("rotated", now);
        }

        _credentials.Add(newCredential);
        UpdatedAt = now;
        RaiseDeviceEvent(
            EventTypes.DeviceCredentialRotated,
            now,
            new() { ["credentialId"] = newCredential.Id, ["hint"] = newCredential.SecretHint });
    }

    public void MarkBrokerSynced(DateTimeOffset now) => BrokerSyncedAt = now;

    public void MarkBrokerUnsynced() => BrokerSyncedAt = null;

    // ---- Presence -------------------------------------------------------------

    public void Connect(
        string sessionId,
        IReadOnlyCollection<string> capabilities,
        IReadOnlyCollection<string> supportedTaskTypes,
        string reportedInfoJson,
        bool busy,
        string? softwareVersion,
        string? protocolVersion,
        DateTimeOffset now)
    {
        if (string.IsNullOrWhiteSpace(sessionId) || sessionId.Length > DomainRules.MaxSessionIdLength)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Invalid sessionId.");
        }

        CurrentSessionId = sessionId;
        Capabilities = capabilities.Where(DomainRules.IsValidCapability).Distinct().Take(DomainRules.MaxCapabilities).ToList();
        SupportedTaskTypes = supportedTaskTypes.Where(DomainRules.IsValidTaskType).Distinct().Take(DomainRules.MaxSupportedTaskTypes).ToList();
        ReportedInfo = reportedInfoJson;
        ReportedBusy = busy;
        PresenceState = PresenceState.Online;
        LastConnectedAt = now;
        LastSeenAt = now;
        UpdatedAt = now;
        RaiseDeviceEvent(
            EventTypes.DeviceConnected,
            now,
            new()
            {
                ["sessionId"] = sessionId,
                ["reason"] = "hello",
                ["softwareVersion"] = softwareVersion,
                ["protocolVersion"] = protocolVersion,
            },
            Actor.Device(Id));
    }

    /// <summary>Cualquier mensaje entrante del device. Un <paramref name="sessionId"/> distinto del actual se ignora.</summary>
    public PresenceChange Touch(string? sessionId, DateTimeOffset now)
    {
        if (sessionId is not null && CurrentSessionId is not null && sessionId != CurrentSessionId)
        {
            return PresenceChange.None;
        }

        LastSeenAt = now;
        switch (PresenceState)
        {
            case PresenceState.Stale:
                PresenceState = PresenceState.Online;
                return PresenceChange.RecoveredFromStale;
            case PresenceState.Offline when CurrentSessionId is not null:
                PresenceState = PresenceState.Online;
                LastConnectedAt = now;
                RaiseDeviceEvent(
                    EventTypes.DeviceConnected,
                    now,
                    new() { ["sessionId"] = CurrentSessionId, ["reason"] = "recovered" },
                    Actor.Device(Id));
                return PresenceChange.RecoveredFromOffline;
            default:
                return PresenceChange.None;
        }
    }

    public PresenceChange Heartbeat(string sessionId, bool busy, DateTimeOffset now)
    {
        if (CurrentSessionId is not null && sessionId != CurrentSessionId)
        {
            return PresenceChange.None;
        }

        ReportedBusy = busy;
        return Touch(sessionId, now);
    }

    /// <summary>goodbye o LWT. Solo aplica si corresponde a la sesión actual.</summary>
    public bool Disconnect(string sessionId, string reason, DateTimeOffset now)
    {
        if (sessionId != CurrentSessionId || PresenceState == PresenceState.Offline)
        {
            return false;
        }

        GoOffline(reason, now, Actor.Device(Id));
        return true;
    }

    public bool MarkStaleIfSilent(DateTimeOffset now, TimeSpan staleAfter)
    {
        if (PresenceState != PresenceState.Online || LastSeenAt is not { } seen || now - seen <= staleAfter)
        {
            return false;
        }

        PresenceState = PresenceState.Stale;
        return true;
    }

    public bool MarkOfflineIfSilent(DateTimeOffset now, TimeSpan offlineAfter)
    {
        if (PresenceState == PresenceState.Offline || LastSeenAt is { } seen && now - seen <= offlineAfter)
        {
            return false;
        }

        GoOffline("heartbeat_timeout", now, Actor.System);
        return true;
    }

    private void ForceOffline(string reason, DateTimeOffset now)
    {
        if (PresenceState != PresenceState.Offline)
        {
            GoOffline(reason, now, null);
        }
    }

    private void GoOffline(string reason, DateTimeOffset now, Actor? actor)
    {
        PresenceState = PresenceState.Offline;
        ReportedBusy = false;
        LastDisconnectedAt = now;
        UpdatedAt = now;
        RaiseDeviceEvent(
            EventTypes.DeviceDisconnected,
            now,
            new() { ["sessionId"] = CurrentSessionId, ["reason"] = reason },
            actor);
    }

    private void EnsureNotRevoked()
    {
        if (AdminState == DeviceAdminState.Revoked)
        {
            throw new DomainException(DomainErrorCodes.DeviceRevoked, "The device is revoked.");
        }
    }

    private static void EnsureValidName(string name)
    {
        if (!DomainRules.IsValidDeviceName(name))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Name must be 1-64 characters without surrounding whitespace.");
        }
    }

    private void RaiseDeviceEvent(string type, DateTimeOffset now, Dictionary<string, object?> data, Actor? actor = null) =>
        Raise(new DomainEvent(type, OwnerId, now, DeviceId: Id, Data: data, Actor: actor));
}
