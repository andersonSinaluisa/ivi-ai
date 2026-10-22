using System.Text.Json;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Events;

namespace Jarvis.Application.Realtime;

/// <summary>Envía notificaciones a los clientes UI del owner (SignalR en la API).</summary>
public interface IRealtimeNotifier
{
    Task NotifyAsync(Guid ownerId, RealtimeNotification notification, CancellationToken cancellationToken);
}

/// <summary>Mapeo Event → notificación realtime (docs/02-domain/event-model.md). Nunca incluye payload ni result.</summary>
public static class RealtimeMapping
{
    public static RealtimeNotification? FromEvent(EventRecord record)
    {
        var data = ParseData(record.Data);
        (string Name, object Payload)? mapped = record.Type switch
        {
            EventTypes.DeviceRegistered => ("device.registered", new { name = Get(data, "name"), type = Get(data, "type") }),
            EventTypes.DeviceUpdated => ("device.updated", new { adminState = (string?)null, changes = GetRaw(data, "changes") }),
            EventTypes.DeviceDisabled => ("device.updated", new { adminState = "Disabled" }),
            EventTypes.DeviceEnabled => ("device.updated", new { adminState = "Active" }),
            EventTypes.DeviceRevoked => ("device.updated", new { adminState = "Revoked" }),
            EventTypes.DeviceConnected => ("device.online", new { presenceState = "Online" }),
            EventTypes.DeviceDisconnected => ("device.offline", new { presenceState = "Offline", reason = Get(data, "reason") }),
            EventTypes.TaskCreated => ("task.created", new { status = "Pending", type = Get(data, "type"), targetDeviceId = Get(data, "targetDeviceId") }),
            EventTypes.TaskDispatched => ("task.dispatched", new { status = "Dispatched", attempt = GetRaw(data, "attempt") }),
            EventTypes.TaskDelivered => ("task.delivered", new { status = "Delivered" }),
            EventTypes.TaskStarted => ("task.started", new { status = "Running" }),
            EventTypes.TaskCompleted => ("task.completed", new { status = "Completed" }),
            EventTypes.TaskFailed => ("task.failed", new { status = "Failed", errorCode = Get(data, "code") }),
            EventTypes.TaskCancelled => ("task.cancelled", new { status = "Cancelled" }),
            EventTypes.TaskExpired => ("task.expired", new { status = "Expired" }),
            EventTypes.ApprovalRequested => ("approval.requested", new { status = "WaitingForApproval", reason = Get(data, "reason") }),
            EventTypes.ApprovalDecisionSent => ("approval.decision_sent", new { decision = Get(data, "decision") }),
            EventTypes.ApprovalGranted => ("approval.resolved", new { decision = "Granted" }),
            EventTypes.ApprovalDenied => ("approval.resolved", new { decision = "Denied" }),
            _ => null,
        };

        return mapped is null
            ? null
            : new RealtimeNotification(
                record.Seq,
                mapped.Value.Name,
                record.OccurredAt,
                record.CorrelationId,
                record.DeviceId,
                record.TaskId,
                JarvisJson.ToElement(mapped.Value.Payload));
    }

    public static RealtimeNotification DeviceStale(Guid deviceId, DateTimeOffset now) =>
        new(null, "device.stale", now, null, deviceId, null, JarvisJson.ToElement(new { presenceState = "Stale" }));

    private static JsonElement ParseData(string json)
    {
        try
        {
            return JsonText.ToElement(json);
        }
        catch (JsonException)
        {
            return JarvisJson.EmptyObject;
        }
    }

    private static string? Get(JsonElement data, string property) =>
        data.ValueKind == JsonValueKind.Object && data.TryGetProperty(property, out var value) && value.ValueKind != JsonValueKind.Null
            ? value.ValueKind == JsonValueKind.String ? value.GetString() : value.GetRawText()
            : null;

    private static JsonElement? GetRaw(JsonElement data, string property) =>
        data.ValueKind == JsonValueKind.Object && data.TryGetProperty(property, out var value) ? value.Clone() : null;
}

/// <summary>HU-042: publica eventos confirmados como notificaciones SignalR.</summary>
public sealed class RealtimeEventSubscriber(IRealtimeNotifier notifier) : IEventSubscriber
{
    public Task HandleAsync(EventRecord record, CancellationToken cancellationToken)
    {
        if (record.OwnerId is not { } ownerId)
        {
            return Task.CompletedTask;
        }

        var notification = RealtimeMapping.FromEvent(record);
        return notification is null ? Task.CompletedTask : notifier.NotifyAsync(ownerId, notification, cancellationToken);
    }
}
