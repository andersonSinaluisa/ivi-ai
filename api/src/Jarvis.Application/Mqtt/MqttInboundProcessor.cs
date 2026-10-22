using System.Collections.Concurrent;
using System.Text.Json;
using System.Threading.RateLimiting;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Presence;
using Jarvis.Application.Tasks;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Mqtt;

/// <summary>Límites en memoria por device: mensajes/s, task.create/min y detección de flood.</summary>
public sealed class DeviceRateLimits : IDisposable
{
    private readonly PartitionedRateLimiter<Guid> _messages;
    private readonly PartitionedRateLimiter<Guid> _taskCreates;
    private readonly ConcurrentDictionary<Guid, FloodState> _flood = new();
    private readonly TimeSpan _floodAfter;

    public DeviceRateLimits(IOptions<SecurityOptions> security, IOptions<TaskOptions> tasks)
    {
        var perSecond = security.Value.MqttMessagesPerSecond;
        var perMinute = tasks.Value.CreateLimitPerMinute;
        _floodAfter = TimeSpan.FromSeconds(security.Value.FloodViolationAfterSeconds);
        _messages = PartitionedRateLimiter.Create<Guid, Guid>(id => RateLimitPartition.GetTokenBucketLimiter(
            id,
            _ => new TokenBucketRateLimiterOptions
            {
                TokenLimit = perSecond,
                TokensPerPeriod = perSecond,
                ReplenishmentPeriod = TimeSpan.FromSeconds(1),
                QueueLimit = 0,
                AutoReplenishment = true,
            }));
        _taskCreates = PartitionedRateLimiter.Create<Guid, Guid>(id => RateLimitPartition.GetFixedWindowLimiter(
            id,
            _ => new FixedWindowRateLimiterOptions { PermitLimit = perMinute, Window = TimeSpan.FromMinutes(1), QueueLimit = 0 }));
    }

    public bool TryAcquireMessage(Guid deviceId)
    {
        using var lease = _messages.AttemptAcquire(deviceId);
        return lease.IsAcquired;
    }

    public bool TryAcquireTaskCreate(Guid deviceId)
    {
        using var lease = _taskCreates.AttemptAcquire(deviceId);
        return lease.IsAcquired;
    }

    /// <summary>Registra un mensaje descartado; true si el exceso persiste y corresponde emitir SECURITY_VIOLATION.</summary>
    public bool RegisterRejection(Guid deviceId, DateTimeOffset now)
    {
        var state = _flood.AddOrUpdate(
            deviceId,
            _ => new FloodState(now, now, null),
            (_, s) => now - s.LastRejected > TimeSpan.FromSeconds(5) ? new FloodState(now, now, s.LastViolation) : s with { LastRejected = now });
        if (now - state.FirstRejected < _floodAfter || (state.LastViolation is { } last && now - last < _floodAfter))
        {
            return false;
        }

        _flood[deviceId] = state with { LastViolation = now };
        return true;
    }

    public void Dispose()
    {
        _messages.Dispose();
        _taskCreates.Dispose();
    }

    private sealed record FloodState(DateTimeOffset FirstRejected, DateTimeOffset LastRejected, DateTimeOffset? LastViolation);
}

/// <summary>
/// HU-022: valida envelopes <c>up</c> (docs/03-protocols/mqtt-protocol.md §3) y los enruta a casos de uso.
/// Excepciones no esperadas se propagan para que el mensaje no se confirme y el broker lo reentregue.
/// </summary>
public sealed class MqttInboundProcessor(
    IServiceScopeFactory scopes,
    DeviceRateLimits limits,
    JarvisMetrics metrics,
    TimeProvider clock,
    IOptions<SecurityOptions> security,
    ILogger<MqttInboundProcessor> logger)
{
    private static readonly Dictionary<string, HashSet<string>> AllowedTypes = new(StringComparer.Ordinal)
    {
        [TopicChannels.Tasks] =
        [
            MessageTypes.TaskAck, MessageTypes.TaskRejected, MessageTypes.TaskStarted, MessageTypes.TaskProgress,
            MessageTypes.TaskApprovalRequired, MessageTypes.TaskCompleted, MessageTypes.TaskFailed, MessageTypes.TaskCancelled,
        ],
        [TopicChannels.Requests] = [MessageTypes.TaskCreate, MessageTypes.TaskGet, MessageTypes.TaskCancelRequest],
        [TopicChannels.Presence] =
        [
            MessageTypes.PresenceHello, MessageTypes.PresenceHeartbeat, MessageTypes.PresenceGoodbye, MessageTypes.PresenceOffline,
        ],
        [TopicChannels.Events] = [MessageTypes.DeviceEvent],
    };

    public async Task ProcessAsync(string topic, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken)
    {
        if (!MqttTopics.TryParse(topic, out var deviceId, out var direction, out var channel) || direction != TopicDirections.Up)
        {
            logger.LogWarning("mqtt_invalid_topic {Topic}", topic);
            return;
        }

        if (payload.Length > MqttEnvelope.MaxMessageBytes)
        {
            Invalid("too_large", deviceId, null);
            return;
        }

        MqttEnvelope envelope;
        try
        {
            envelope = MqttEnvelope.Parse(payload.Span);
        }
        catch (JsonException)
        {
            Invalid("json", deviceId, null);
            return;
        }

        if (envelope.V != MqttEnvelope.CurrentVersion || envelope.Id is null || envelope.Type is null || envelope.Ts is null
            || envelope.DeviceId is null || envelope.Data.ValueKind != JsonValueKind.Object)
        {
            Invalid("envelope", deviceId, envelope.Type);
            return;
        }

        if (!AllowedTypes.TryGetValue(channel, out var types) || !types.Contains(envelope.Type))
        {
            metrics.MqttReceived(envelope.Type.Length <= 40 ? envelope.Type : "other", "unknown_type");
            logger.LogInformation("mqtt_unknown_type {MessageType} on {Channel} from {DeviceId}", envelope.Type, channel, deviceId);
            return;
        }

        if (channel == TopicChannels.Events)
        {
            metrics.MqttReceived(envelope.Type, "ignored");
            logger.LogDebug("device.event from {DeviceId} received and discarded (reserved)", deviceId);
            return;
        }

        var now = clock.GetUtcNow();
        if (!limits.TryAcquireMessage(deviceId))
        {
            metrics.RateLimited("mqtt_messages");
            metrics.MqttReceived(envelope.Type, "rate_limited");
            logger.LogWarning("MQTT message from {DeviceId} dropped by rate limit", deviceId);
            if (limits.RegisterRejection(deviceId, now))
            {
                await RecordViolationAsync(deviceId, SecurityViolations.MessageFlood, envelope, cancellationToken);
            }

            return;
        }

        await using var scope = scopes.CreateAsyncScope();
        var services = scope.ServiceProvider;
        var context = services.GetRequiredService<IRequestContext>();
        context.EnterSystemScope();
        context.CorrelationId = envelope.CorrelationId ?? Guid.CreateVersion7();

        using var logScope = logger.BeginScope(new Dictionary<string, object?>
        {
            ["DeviceId"] = deviceId,
            ["MqttMessageId"] = envelope.Id,
            ["MqttMessageType"] = envelope.Type,
            ["CorrelationId"] = context.CorrelationId,
        });

        var db = services.GetRequiredService<IJarvisDbContext>();
        var device = await db.Devices.FirstOrDefaultAsync(d => d.Id == deviceId, cancellationToken);
        if (device is null)
        {
            Invalid("unknown_device", deviceId, envelope.Type);
            return;
        }

        if (envelope.DeviceId != deviceId)
        {
            SecurityViolations.Record(db, device, SecurityViolations.TopicDeviceMismatch, now, envelope.CorrelationId);
            await db.SaveChangesAsync(cancellationToken);
            metrics.MqttReceived(envelope.Type, "rejected");
            logger.LogWarning("Envelope deviceId does not match topic device {DeviceId}", deviceId);
            return;
        }

        if (!device.IsActive)
        {
            metrics.MqttReceived(envelope.Type, "rejected");
            logger.LogWarning("Message from non-active device {DeviceId} discarded", deviceId);
            return;
        }

        context.Principal = new Principal(PrincipalType.Device, device.Id, device.OwnerId);

        try
        {
            if (channel == TopicChannels.Requests
                && (envelope.Ts.Value - now).Duration() > TimeSpan.FromSeconds(security.Value.MessageTimestampToleranceSeconds))
            {
                device.Touch(null, now);
                await db.SaveChangesAsync(cancellationToken);
                await services.GetRequiredService<DeviceRequestHandler>()
                    .RejectAsync(device, envelope, envelope.Type, ErrorCodes.StaleMessage, "Message timestamp outside the accepted window.", cancellationToken);
                metrics.MqttReceived(envelope.Type, "rejected");
                return;
            }

            if (envelope.Type is not (MessageTypes.PresenceHello or MessageTypes.PresenceHeartbeat
                or MessageTypes.PresenceGoodbye or MessageTypes.PresenceOffline))
            {
                device.Touch(null, now);
                await db.SaveChangesAsync(cancellationToken);
            }

            switch (channel)
            {
                case TopicChannels.Tasks:
                    await services.GetRequiredService<DeviceTaskUpdateHandler>().HandleAsync(device, envelope, cancellationToken);
                    break;
                case TopicChannels.Requests:
                    var limited = envelope.Type == MessageTypes.TaskCreate && !limits.TryAcquireTaskCreate(deviceId);
                    if (limited)
                    {
                        metrics.RateLimited("mqtt_task_create");
                    }

                    await services.GetRequiredService<DeviceRequestHandler>().HandleAsync(device, envelope, limited, cancellationToken);
                    break;
                case TopicChannels.Presence:
                    var presence = services.GetRequiredService<PresenceHandler>();
                    await (envelope.Type switch
                    {
                        MessageTypes.PresenceHello => presence.HandleHelloAsync(device, envelope, cancellationToken),
                        MessageTypes.PresenceHeartbeat => presence.HandleHeartbeatAsync(device, envelope, cancellationToken),
                        _ => presence.HandleDisconnectAsync(device, envelope, cancellationToken),
                    });
                    break;
            }

            metrics.MqttReceived(envelope.Type, "ok");
        }
        catch (JsonException exception)
        {
            metrics.MqttReceived(envelope.Type, "invalid");
            logger.LogWarning(exception, "mqtt_invalid_envelope: data of {MessageType} is not valid", envelope.Type);
        }
    }

    private void Invalid(string reason, Guid deviceId, string? type)
    {
        metrics.MqttReceived(type is { Length: <= 40 } ? type : "unknown", "invalid");
        logger.LogWarning("mqtt_invalid_envelope ({Reason}) from {DeviceId}", reason, deviceId);
    }

    private async Task RecordViolationAsync(Guid deviceId, string rule, MqttEnvelope envelope, CancellationToken cancellationToken)
    {
        await using var scope = scopes.CreateAsyncScope();
        scope.ServiceProvider.GetRequiredService<IRequestContext>().EnterSystemScope();
        var db = scope.ServiceProvider.GetRequiredService<IJarvisDbContext>();
        var device = await db.Devices.FirstOrDefaultAsync(d => d.Id == deviceId, cancellationToken);
        if (device is not null)
        {
            SecurityViolations.Record(db, device, rule, clock.GetUtcNow(), envelope.CorrelationId);
            await db.SaveChangesAsync(cancellationToken);
        }
    }
}
