using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Realtime;
using Jarvis.Application.Tasks;
using Jarvis.Contracts.Mqtt;
using Jarvis.Domain.Devices;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Presence;

/// <summary>HU-035 / HU-036: hello, heartbeat, goodbye y LWT (docs/03-protocols/mqtt-protocol.md §6).</summary>
public sealed class PresenceHandler(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    DispatchSignal dispatchSignal,
    TimeProvider clock,
    IOptions<PresenceOptions> options,
    ILogger<PresenceHandler> logger)
{
    public const int MaxOpenTaskIds = 256;

    public async Task HandleHelloAsync(Device device, MqttEnvelope envelope, CancellationToken cancellationToken)
    {
        var data = envelope.ReadData<PresenceHelloData>();
        var now = clock.GetUtcNow();
        var reportedInfo = JsonText.Serialize(new
        {
            softwareVersion = data.SoftwareVersion,
            protocolVersion = data.ProtocolVersion,
            platform = data.Platform,
        });

        device.Connect(
            data.SessionId,
            data.Capabilities ?? [],
            data.SupportedTaskTypes ?? [],
            reportedInfo,
            data.Busy,
            data.SoftwareVersion,
            data.ProtocolVersion,
            now);

        var reported = data.OpenTaskIds ?? [];
        if (reported.Count > MaxOpenTaskIds)
        {
            logger.LogWarning("Device {DeviceId} reported {Count} open tasks; only the first {Max} are processed", device.Id, reported.Count, MaxOpenTaskIds);
        }

        var open = reported.Take(MaxOpenTaskIds).ToHashSet();
        var active = await db.Tasks
            .Where(t => t.TargetDeviceId == device.Id && TaskMapper.ActiveStatuses.Contains(t.Status))
            .ToListAsync(cancellationToken);

        var reconciled = 0;
        foreach (var task in active.Where(t => !open.Contains(t.Id)))
        {
            if (task.HandleMissingOnDevice(now))
            {
                reconciled++;
            }
        }

        if (open.Count > 0)
        {
            var ids = open.ToList();
            var known = await db.Tasks.CountAsync(t => ids.Contains(t.Id) && t.TargetDeviceId == device.Id, cancellationToken);
            if (known != open.Count)
            {
                logger.LogWarning("Device {DeviceId} reported {Unknown} open task ids that do not belong to it", device.Id, open.Count - known);
            }
        }

        await db.SaveChangesAsync(cancellationToken);
        logger.LogInformation(
            "Device {DeviceId} connected (session {SessionId}); {Reconciled} tasks reconciled",
            device.Id,
            data.SessionId,
            reconciled);

        await publisher.PublishAsync(
            device.Id,
            MessageTypes.PresenceHelloAck,
            new PresenceHelloAckData(data.SessionId, now, options.Value.HeartbeatIntervalSeconds, MqttEnvelope.MaxMessageBytes),
            envelope.CorrelationId,
            envelope.Id,
            cancellationToken);
        dispatchSignal.Wake();
    }

    public async Task HandleHeartbeatAsync(Device device, MqttEnvelope envelope, CancellationToken cancellationToken)
    {
        var data = envelope.ReadData<PresenceHeartbeatData>();
        device.Heartbeat(data.SessionId, data.Busy, clock.GetUtcNow());
        await db.SaveChangesAsync(cancellationToken);
    }

    public async Task HandleDisconnectAsync(Device device, MqttEnvelope envelope, CancellationToken cancellationToken)
    {
        var goodbye = envelope.Type == MessageTypes.PresenceGoodbye;
        var sessionId = goodbye
            ? envelope.ReadData<PresenceGoodbyeData>().SessionId
            : envelope.ReadData<PresenceOfflineData>().SessionId;

        if (device.Disconnect(sessionId, goodbye ? "goodbye" : "lwt", clock.GetUtcNow()))
        {
            await db.SaveChangesAsync(cancellationToken);
            logger.LogInformation("Device {DeviceId} offline ({Reason})", device.Id, goodbye ? "goodbye" : "lwt");
        }
        else
        {
            logger.LogDebug("Ignored {MessageType} for old session of {DeviceId}", envelope.Type, device.Id);
        }
    }
}

/// <summary>HU-037: Online → Stale → Offline para devices silenciosos.</summary>
public sealed class PresenceSweepService(
    IJarvisDbContext db,
    IRealtimeNotifier realtime,
    TimeProvider clock,
    IOptions<PresenceOptions> options,
    ILogger<PresenceSweepService> logger)
{
    public async Task<int> SweepAsync(CancellationToken cancellationToken)
    {
        var now = clock.GetUtcNow();
        var staleAfter = TimeSpan.FromSeconds(options.Value.StaleAfterSeconds);
        var offlineAfter = TimeSpan.FromSeconds(options.Value.OfflineAfterSeconds);
        var threshold = now - staleAfter;
        var devices = await db.Devices
            .Where(d => d.PresenceState != PresenceState.Offline && (d.LastSeenAt == null || d.LastSeenAt < threshold))
            .ToListAsync(cancellationToken);

        var stale = new List<Device>();
        var changed = 0;
        foreach (var device in devices)
        {
            if (device.MarkOfflineIfSilent(now, offlineAfter))
            {
                changed++;
            }
            else if (device.MarkStaleIfSilent(now, staleAfter))
            {
                stale.Add(device);
                changed++;
            }
        }

        if (changed == 0)
        {
            return 0;
        }

        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (Exception exception) when (db.IsConcurrencyConflict(exception))
        {
            db.ResetTracking();
            return 0;
        }

        foreach (var device in stale)
        {
            await realtime.NotifyAsync(device.OwnerId, RealtimeMapping.DeviceStale(device.Id, now), cancellationToken);
        }

        logger.LogInformation("Presence sweep: {Changed} devices changed state", changed);
        return changed;
    }
}
