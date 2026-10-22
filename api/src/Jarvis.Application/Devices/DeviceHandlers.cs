using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Security;
using Jarvis.Contracts;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Tasks;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Devices;

public static class DeviceMapper
{
    public static DeviceDto ToDto(Device device)
    {
        var credential = device.ActiveCredential;
        return new DeviceDto(
            device.Id,
            device.Name,
            device.Type.ToString(),
            device.Status.ToString(),
            device.AdminState.ToString(),
            device.PresenceState.ToString(),
            device.ReportedBusy,
            device.Capabilities,
            device.SupportedTaskTypes,
            JsonText.ToElement(device.ReportedInfo),
            device.LastSeenAt,
            device.LastConnectedAt,
            device.LastDisconnectedAt,
            device.BrokerSyncedAt is not null,
            credential is null ? null : new CredentialSummaryDto(credential.Id, credential.SecretHint, credential.CreatedAt, credential.LastUsedAt),
            device.CreatedAt,
            device.UpdatedAt);
    }

    public static bool TryParseType(string? value, out DeviceType type)
    {
        type = default;
        return value is not null
            && Enum.TryParse(value, ignoreCase: true, out type)
            && Enum.IsDefined(type)
            && !int.TryParse(value, out _);
    }

    public static bool TryParseStatus(string? value, out DeviceStatus status)
    {
        status = default;
        return value is not null && Enum.TryParse(value, ignoreCase: true, out status) && Enum.IsDefined(status) && !int.TryParse(value, out _);
    }
}

/// <summary>Carga devices del owner del principal (con defensa explícita además del filtro global).</summary>
internal static class DeviceQueries
{
    public static Task<Device?> FindOwnedAsync(this IJarvisDbContext db, Principal principal, Guid id, CancellationToken ct, bool includeCredentials = true)
    {
        var query = db.Devices.Where(d => d.Id == id && d.OwnerId == principal.OwnerId);
        if (includeCredentials)
        {
            query = query.Include(d => d.Credentials);
        }

        return query.FirstOrDefaultAsync(ct);
    }
}

public sealed class RegisterDeviceHandler(
    IJarvisDbContext db,
    IDeviceSecretHasher hasher,
    IBrokerCredentialAdmin broker,
    TimeProvider clock,
    IOptions<JarvisOptions> options,
    IOptions<MqttOptions> mqttOptions,
    ILogger<RegisterDeviceHandler> logger)
{
    public async Task<Result<RegisterDeviceResponse>> HandleAsync(Principal principal, CreateDeviceRequest request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        if (!DomainRules.IsValidDeviceName(request.Name))
        {
            return AppError.Validation("name", "Name must be 1-64 characters without surrounding whitespace.");
        }

        if (!DeviceMapper.TryParseType(request.Type, out var type))
        {
            return AppError.Validation("type", "Type must be one of Windows, ESP32, Mobile, Web, Unknown.");
        }

        var ownerId = principal.OwnerId;
        var count = await db.Devices.CountAsync(d => d.OwnerId == ownerId, cancellationToken);
        if (count >= options.Value.MaxDevicesPerOwner)
        {
            return new AppError(ErrorCodes.DeviceLimitReached, $"Maximum of {options.Value.MaxDevicesPerOwner} devices per owner reached.");
        }

        if (await db.Devices.AnyAsync(d => d.OwnerId == ownerId && d.Name == request.Name, cancellationToken))
        {
            return NameTaken();
        }

        var now = clock.GetUtcNow();
        var deviceId = Guid.CreateVersion7(now);
        var secret = SecretGenerator.NewDeviceSecret();
        var credential = DeviceCredential.CreateSecret(Guid.CreateVersion7(now), deviceId, hasher.Hash(secret), SecretGenerator.Hint(secret), now);
        var device = Device.Register(deviceId, ownerId, request.Name!, type, credential, now);

        try
        {
            await db.InTransactionAsync(
                async ct =>
                {
                    db.Devices.Add(device);
                    await db.SaveChangesAsync(ct);
                    await broker.CreateClientAsync(deviceId, secret, ct);
                    device.MarkBrokerSynced(clock.GetUtcNow());
                    await db.SaveChangesAsync(ct);
                    return true;
                },
                cancellationToken);
        }
        catch (Exception exception) when (exception is BrokerUnavailableException or BrokerCommandException)
        {
            db.ResetTracking();
            logger.LogWarning(exception, "Broker unavailable while registering device; nothing persisted");
            return new AppError(ErrorCodes.ServiceUnavailable, "Message broker unavailable. Nothing was created; retry later.");
        }
        catch (Exception exception) when (db.IsUniqueViolation(exception))
        {
            db.ResetTracking();
            return NameTaken();
        }

        logger.LogInformation("Device {DeviceId} registered as {DeviceType}", deviceId, type);
        return new RegisterDeviceResponse(DeviceMapper.ToDto(device), IssuedCredential(deviceId, secret, mqttOptions.Value));
    }

    internal static IssuedCredentialDto IssuedCredential(Guid deviceId, string secret, MqttOptions mqtt)
    {
        var id = deviceId.ToString("D");
        return new IssuedCredentialDto(
            deviceId,
            secret,
            new MqttConnectionInfo(mqtt.PublicHost, mqtt.PublicPort, id, id),
            "This secret is shown only once. Store it securely.");
    }

    private static AppError NameTaken() => new(ErrorCodes.DeviceNameTaken, "A device with this name already exists.");
}

public sealed record ListDevicesQuery(string? Type, string? Status, int? Limit, string? Cursor);

public sealed class DeviceQueryHandler(IJarvisDbContext db)
{
    public async Task<Result<Page<DeviceDto>>> ListAsync(Principal principal, ListDevicesQuery query, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var ownerId = principal.OwnerId;
        var devices = db.Devices.AsNoTracking().Include(d => d.Credentials).Where(d => d.OwnerId == ownerId);

        if (query.Type is not null)
        {
            if (!DeviceMapper.TryParseType(query.Type, out var type))
            {
                return AppError.Validation("type", "Unknown device type.");
            }

            devices = devices.Where(d => d.Type == type);
        }

        if (query.Status is not null)
        {
            if (!DeviceMapper.TryParseStatus(query.Status, out var status))
            {
                return AppError.Validation("status", "Unknown device status.");
            }

            devices = status switch
            {
                DeviceStatus.Disabled => devices.Where(d => d.AdminState != DeviceAdminState.Active),
                DeviceStatus.Offline => devices.Where(d => d.AdminState == DeviceAdminState.Active && d.PresenceState == PresenceState.Offline),
                DeviceStatus.Stale => devices.Where(d => d.AdminState == DeviceAdminState.Active && d.PresenceState == PresenceState.Stale),
                DeviceStatus.Busy => devices.Where(d => d.AdminState == DeviceAdminState.Active && d.PresenceState == PresenceState.Online && d.ReportedBusy),
                _ => devices.Where(d => d.AdminState == DeviceAdminState.Active && d.PresenceState == PresenceState.Online && !d.ReportedBusy),
            };
        }

        if (query.Cursor is not null)
        {
            if (!KeysetCursor.TryDecode(query.Cursor, out var createdAt, out var id))
            {
                return AppError.Validation("cursor", "Invalid cursor.");
            }

            devices = devices.Where(d => d.CreatedAt > createdAt || (d.CreatedAt == createdAt && d.Id.CompareTo(id) > 0));
        }

        var limit = Limits.ClampLimit(query.Limit, 50, 200);
        var items = await devices.OrderBy(d => d.CreatedAt).ThenBy(d => d.Id).Take(limit + 1).ToListAsync(cancellationToken);
        var next = items.Count > limit ? KeysetCursor.Encode(items[limit - 1].CreatedAt, items[limit - 1].Id) : null;
        return new Page<DeviceDto>(items.Take(limit).Select(DeviceMapper.ToDto).ToList(), next);
    }

    public async Task<Result<DeviceDto>> GetAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var device = await db.FindOwnedAsync(principal, id, cancellationToken);
        return device is null ? AppError.NotFound("Device") : DeviceMapper.ToDto(device);
    }
}

/// <summary>HU-014 / HU-015 / HU-016: administración de devices.</summary>
public sealed class DeviceAdminHandler(
    IJarvisDbContext db,
    IDeviceSecretHasher hasher,
    IBrokerCredentialAdmin broker,
    PrincipalStatusService principalStatus,
    TimeProvider clock,
    IOptions<MqttOptions> mqttOptions,
    ILogger<DeviceAdminHandler> logger)
{
    public async Task<Result<DeviceDto>> RenameAsync(Principal principal, Guid id, string? name, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var device = await db.FindOwnedAsync(principal, id, cancellationToken);
        if (device is null)
        {
            return AppError.NotFound("Device");
        }

        if (!DomainRules.IsValidDeviceName(name))
        {
            return AppError.Validation("name", "Name must be 1-64 characters without surrounding whitespace.");
        }

        if (device.Name != name && await db.Devices.AnyAsync(d => d.OwnerId == principal.OwnerId && d.Name == name && d.Id != id, cancellationToken))
        {
            return new AppError(ErrorCodes.DeviceNameTaken, "A device with this name already exists.");
        }

        try
        {
            device.Rename(name!, clock.GetUtcNow());
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }
        catch (Exception exception) when (db.IsUniqueViolation(exception))
        {
            db.ResetTracking();
            return new AppError(ErrorCodes.DeviceNameTaken, "A device with this name already exists.");
        }

        return DeviceMapper.ToDto(device);
    }

    public async Task<Result<DeviceDto>> DisableAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        var loaded = await LoadForAdminAsync(principal, id, cancellationToken);
        if (!loaded.IsSuccess)
        {
            return loaded.Error!;
        }

        var device = loaded.Value;
        try
        {
            if (device.Disable(clock.GetUtcNow()))
            {
                await db.SaveChangesAsync(cancellationToken);
                principalStatus.Invalidate(device.Id);
                await TryBrokerAsync(ct => broker.DisableClientAsync(device.Id, ct), device, "disable", cancellationToken);
            }
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        return DeviceMapper.ToDto(device);
    }

    public async Task<Result<DeviceDto>> EnableAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        var loaded = await LoadForAdminAsync(principal, id, cancellationToken);
        if (!loaded.IsSuccess)
        {
            return loaded.Error!;
        }

        var device = loaded.Value;
        try
        {
            if (device.Enable(clock.GetUtcNow()))
            {
                await db.SaveChangesAsync(cancellationToken);
                principalStatus.Invalidate(device.Id);
                await TryBrokerAsync(ct => broker.EnableClientAsync(device.Id, ct), device, "enable", cancellationToken);
            }
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        return DeviceMapper.ToDto(device);
    }

    public async Task<Result<DeviceDto>> RevokeAsync(Principal principal, Guid id, string? reason, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var device = await db.FindOwnedAsync(principal, id, cancellationToken);
        if (device is null)
        {
            return AppError.NotFound("Device");
        }

        if (device.AdminState == DeviceAdminState.Revoked)
        {
            return DeviceMapper.ToDto(device);
        }

        var now = clock.GetUtcNow();
        var activeTasks = await db.Tasks
            .Where(t => t.TargetDeviceId == id && t.Status != JarvisTaskStatus.Completed && t.Status != JarvisTaskStatus.Failed
                        && t.Status != JarvisTaskStatus.Cancelled && t.Status != JarvisTaskStatus.Expired)
            .ToListAsync(cancellationToken);
        var cancelled = activeTasks.Count(t => t.CancelByRevocation(now));

        var grants = await db.DeviceTaskGrants.Where(g => g.SourceDeviceId == id || g.TargetDeviceId == id).ToListAsync(cancellationToken);
        foreach (var grant in grants)
        {
            grant.MarkDeleted(now);
            db.DeviceTaskGrants.Remove(grant);
        }

        device.Revoke(DomainRules.Truncate(reason, 200), cancelled, now);
        await db.SaveChangesAsync(cancellationToken);
        principalStatus.Invalidate(device.Id);
        await TryBrokerAsync(ct => broker.DeleteClientAsync(device.Id, ct), device, "delete", cancellationToken);
        logger.LogInformation("Device {DeviceId} revoked; {CancelledTasks} tasks cancelled", device.Id, cancelled);
        return DeviceMapper.ToDto(device);
    }

    public async Task<Result<IssuedCredentialDto>> RotateCredentialAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        var loaded = await LoadForAdminAsync(principal, id, cancellationToken);
        if (!loaded.IsSuccess)
        {
            return loaded.Error!;
        }

        var device = loaded.Value;
        var now = clock.GetUtcNow();
        var secret = SecretGenerator.NewDeviceSecret();
        var credential = DeviceCredential.CreateSecret(Guid.CreateVersion7(now), device.Id, hasher.Hash(secret), SecretGenerator.Hint(secret), now);

        try
        {
            await db.InTransactionAsync(
                async ct =>
                {
                    device.RotateCredential(credential, now);
                    db.DeviceCredentials.Add(credential);
                    await db.SaveChangesAsync(ct);
                    await broker.SetPasswordAsync(device.Id, secret, ct);

                    // Cambiar la contraseña no cierra sesiones existentes: disable + enable fuerza la desconexión.
                    await broker.DisableClientAsync(device.Id, ct);
                    if (device.AdminState == DeviceAdminState.Active)
                    {
                        await broker.EnableClientAsync(device.Id, ct);
                    }

                    device.MarkBrokerSynced(clock.GetUtcNow());
                    await db.SaveChangesAsync(ct);
                    return true;
                },
                cancellationToken);
        }
        catch (Exception exception) when (exception is BrokerUnavailableException or BrokerCommandException)
        {
            db.ResetTracking();
            logger.LogWarning(exception, "Broker unavailable while rotating credential of {DeviceId}; nothing changed", device.Id);
            return new AppError(ErrorCodes.ServiceUnavailable, "Message broker unavailable. Credential was not rotated; retry later.");
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        principalStatus.Invalidate(device.Id);
        return RegisterDeviceHandler.IssuedCredential(device.Id, secret, mqttOptions.Value);
    }

    private async Task<Result<Device>> LoadForAdminAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var device = await db.FindOwnedAsync(principal, id, cancellationToken);
        if (device is null)
        {
            return AppError.NotFound("Device");
        }

        return device.AdminState == DeviceAdminState.Revoked
            ? new AppError(ErrorCodes.DeviceRevoked, "The device is revoked.")
            : device;
    }

    /// <summary>
    /// Quitar acceso nunca depende del broker: si falla, el cambio queda en BD y la reconciliación lo aplica.
    /// </summary>
    private async Task TryBrokerAsync(Func<CancellationToken, Task> action, Device device, string operation, CancellationToken cancellationToken)
    {
        try
        {
            await action(cancellationToken);
        }
        catch (Exception exception) when (exception is BrokerUnavailableException or BrokerCommandException)
        {
            logger.LogWarning(exception, "Broker {Operation} failed for {DeviceId}; reconciliation will apply it", operation, device.Id);
        }
    }
}
