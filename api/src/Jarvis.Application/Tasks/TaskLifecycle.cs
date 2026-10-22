using System.Collections.Concurrent;
using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Tasks;

public static class SecurityViolations
{
    public const string TopicDeviceMismatch = "topic_device_mismatch";
    public const string NotTaskTarget = "not_task_target";
    public const string UnknownTask = "unknown_task";
    public const string TaskNotDispatched = "task_not_dispatched";
    public const string MessageFlood = "message_flood";

    public static void Record(IJarvisDbContext db, Device device, string rule, DateTimeOffset now, Guid? correlationId, Guid? taskId = null) =>
        db.AddEvent(new DomainEvent(
            EventTypes.SecurityViolation,
            device.OwnerId,
            now,
            DeviceId: device.Id,
            TaskId: taskId,
            Data: new Dictionary<string, object?> { ["rule"] = rule },
            Actor: Actor.Device(device.Id),
            CorrelationId: correlationId));
}

/// <summary>HU-028: publica tareas Pending a targets Online. Reclamo con FOR UPDATE SKIP LOCKED.</summary>
public sealed class TaskDispatchService(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    TimeProvider clock,
    IOptions<TaskOptions> options,
    ILogger<TaskDispatchService> logger)
{
    private static readonly ConcurrentDictionary<Guid, DateTimeOffset> LastCancelSent = new();

    public async Task<int> DispatchPendingAsync(CancellationToken cancellationToken)
    {
        var now = clock.GetUtcNow();
        var claimed = await db.InTransactionAsync(
            async ct =>
            {
                var tasks = await db.ClaimDispatchableTasksAsync(now, options.Value.DispatchBatchSize, ct);
                foreach (var task in tasks)
                {
                    task.Dispatch(now);
                }

                if (tasks.Count > 0)
                {
                    await db.SaveChangesAsync(ct);
                }

                return tasks;
            },
            cancellationToken);

        foreach (var task in claimed)
        {
            var published = await publisher.PublishAsync(
                task.TargetDeviceId,
                MessageTypes.TaskDispatch,
                TaskMapper.ToDispatch(task),
                task.CorrelationId,
                null,
                cancellationToken);
            if (published)
            {
                logger.LogInformation("Task {TaskId} dispatched to {DeviceId} (attempt {Attempt})", task.Id, task.TargetDeviceId, task.AttemptCount);
            }
            else
            {
                // El ACK timeout la devolverá a Pending.
                logger.LogWarning("Task {TaskId} marked Dispatched but publish failed; ack timeout will retry", task.Id);
            }
        }

        return claimed.Count;
    }

    /// <summary>Reenvía task.cancel a tareas con cancelación pendiente (docs/04-data/persistence-strategy.md).</summary>
    public async Task ResendPendingCancellationsAsync(CancellationToken cancellationToken)
    {
        var now = clock.GetUtcNow();
        var every = TimeSpan.FromSeconds(options.Value.CancelResendSeconds);
        var tasks = await db.Tasks.AsNoTracking()
            .Where(t => t.CancelRequestedAt != null && TaskMapper.ActiveStatuses.Contains(t.Status) && t.CancelRequestedAt < now - every)
            .Take(200)
            .ToListAsync(cancellationToken);

        foreach (var task in tasks)
        {
            if (LastCancelSent.TryGetValue(task.Id, out var last) && now - last < every)
            {
                continue;
            }

            LastCancelSent[task.Id] = now;
            await publisher.PublishAsync(
                task.TargetDeviceId,
                MessageTypes.TaskCancel,
                new TaskCancelData(task.Id, TaskErrorCodes.UserRequest),
                task.CorrelationId,
                null,
                cancellationToken);
        }

        foreach (var key in LastCancelSent.Where(p => now - p.Value > every * 10).Select(p => p.Key).ToList())
        {
            LastCancelSent.TryRemove(key, out _);
        }
    }
}

/// <summary>HU-030 / HU-031: timeouts de ACK, expiración y duración máxima.</summary>
public sealed class TaskSweepService(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    DispatchSignal dispatchSignal,
    TimeProvider clock,
    IOptions<TaskOptions> options,
    ILogger<TaskSweepService> logger)
{
    public async Task<int> SweepAckTimeoutsAsync(CancellationToken cancellationToken)
    {
        var now = clock.GetUtcNow();
        var threshold = now - TimeSpan.FromSeconds(options.Value.AckTimeoutSeconds);
        var tasks = await db.Tasks
            .Where(t => t.Status == JarvisTaskStatus.Dispatched && t.DispatchedAt < threshold)
            .OrderBy(t => t.DispatchedAt)
            .Take(500)
            .ToListAsync(cancellationToken);

        var changed = tasks.Count(t => t.HandleAckTimeout(now));
        if (changed == 0)
        {
            return 0;
        }

        await SaveIgnoringConflictsAsync(cancellationToken);
        logger.LogInformation("{Count} dispatched tasks without ack were reverted or failed", changed);
        dispatchSignal.Wake();
        return changed;
    }

    public async Task<int> SweepExpirationsAsync(CancellationToken cancellationToken)
    {
        var now = clock.GetUtcNow();
        var expirable = await db.Tasks
            .Where(t => (t.Status == JarvisTaskStatus.Pending || t.Status == JarvisTaskStatus.Dispatched) && t.ExpiresAt < now)
            .Take(500)
            .ToListAsync(cancellationToken);
        var expired = expirable.Count(t => t.ExpireIfDue(now));

        var executing = await db.Tasks
            .Where(t => TaskMapper.ExecutingStatuses.Contains(t.Status) && t.DeliveredAt != null)
            .OrderBy(t => t.DeliveredAt)
            .Take(500)
            .ToListAsync(cancellationToken);
        var timedOut = executing.Where(t => t.TimeoutIfDue(now)).ToList();

        if (expired + timedOut.Count == 0)
        {
            return 0;
        }

        await SaveIgnoringConflictsAsync(cancellationToken);
        foreach (var task in timedOut.Where(t => t.ErrorCode == TaskErrorCodes.ExecutionTimeout))
        {
            await publisher.PublishAsync(
                task.TargetDeviceId,
                MessageTypes.TaskCancel,
                new TaskCancelData(task.Id, TaskErrorCodes.ExecutionTimeout),
                task.CorrelationId,
                null,
                cancellationToken);
        }

        logger.LogInformation("{Expired} tasks expired, {TimedOut} tasks timed out", expired, timedOut.Count);
        return expired + timedOut.Count;
    }

    private async Task SaveIgnoringConflictsAsync(CancellationToken cancellationToken)
    {
        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (Exception exception) when (db.IsConcurrencyConflict(exception))
        {
            // Un mensaje del device ganó la carrera; el próximo ciclo reevalúa.
            db.ResetTracking();
            logger.LogInformation("Sweep lost a concurrency race; will retry next cycle");
        }
    }
}

/// <summary>HU-029: aplica mensajes <c>up/tasks</c> a la máquina de estados.</summary>
public sealed class DeviceTaskUpdateHandler(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    TimeProvider clock,
    ILogger<DeviceTaskUpdateHandler> logger)
{
    private static readonly HashSet<string> TerminalMessages =
    [
        MessageTypes.TaskRejected, MessageTypes.TaskCompleted, MessageTypes.TaskFailed, MessageTypes.TaskCancelled,
    ];

    public async Task HandleAsync(Device device, MqttEnvelope envelope, CancellationToken cancellationToken)
    {
        var type = envelope.Type!;
        var taskId = envelope.ReadData<TaskRefData>().TaskId;
        var now = clock.GetUtcNow();

        TaskUpdateOutcome outcome = TaskUpdateOutcome.Duplicate;
        JarvisTask? task = null;
        for (var attempt = 0; attempt < 3; attempt++)
        {
            task = await db.Tasks.FirstOrDefaultAsync(t => t.Id == taskId, cancellationToken);
            if (task is null)
            {
                SecurityViolations.Record(db, device, SecurityViolations.UnknownTask, now, envelope.CorrelationId, taskId);
                await db.SaveChangesAsync(cancellationToken);
                logger.LogWarning("Device {DeviceId} sent {MessageType} for unknown task {TaskId}", device.Id, type, taskId);
                return;
            }

            if (task.TargetDeviceId != device.Id)
            {
                SecurityViolations.Record(db, device, SecurityViolations.NotTaskTarget, now, envelope.CorrelationId, taskId);
                await db.SaveChangesAsync(cancellationToken);
                logger.LogWarning("Device {DeviceId} is not the target of task {TaskId}", device.Id, taskId);
                return;
            }

            outcome = Apply(task, envelope, type, now);
            try
            {
                await db.SaveChangesAsync(cancellationToken);
                break;
            }
            catch (Exception exception) when (db.IsConcurrencyConflict(exception) && attempt < 2)
            {
                db.ResetTracking();
            }
        }

        switch (outcome)
        {
            case TaskUpdateOutcome.SecurityViolation:
                SecurityViolations.Record(db, device, SecurityViolations.TaskNotDispatched, now, envelope.CorrelationId, taskId);
                await db.SaveChangesAsync(cancellationToken);
                logger.LogWarning("Device {DeviceId} reported on task {TaskId} that was never dispatched", device.Id, taskId);
                return;
            case TaskUpdateOutcome.Terminal when TerminalMessages.Contains(type):
                logger.LogWarning(
                    "late_terminal_update: task {TaskId} is {Status}, device reported {MessageType}",
                    task!.Id,
                    task.Status,
                    type);
                break;
            case TaskUpdateOutcome.Applied:
                logger.LogInformation("Task {TaskId} is now {Status} after {MessageType}", task!.Id, task.Status, type);
                break;
            default:
                logger.LogDebug("task_update_ignored: {Outcome} for task {TaskId} ({MessageType})", outcome, taskId, type);
                break;
        }

        if (TerminalMessages.Contains(type) && task!.IsTerminal)
        {
            await publisher.PublishAsync(
                device.Id,
                MessageTypes.TaskUpdateAck,
                new TaskUpdateAckData(task.Id, task.Status.ToString()),
                task.CorrelationId,
                envelope.Id,
                cancellationToken);
        }
    }

    private static TaskUpdateOutcome Apply(JarvisTask task, MqttEnvelope envelope, string type, DateTimeOffset now)
    {
        switch (type)
        {
            case MessageTypes.TaskAck:
                return task.Acknowledge(now);
            case MessageTypes.TaskRejected:
                var rejected = envelope.ReadData<TaskRejectedData>();
                return task.Reject(ToError(rejected.Error), now);
            case MessageTypes.TaskStarted:
                return task.Start(now);
            case MessageTypes.TaskProgress:
                var progress = envelope.ReadData<TaskProgressData>();
                return task.ReportProgress(progress.Percent is { } p ? (short)Math.Clamp(p, -1, 101) : null, progress.Message, now);
            case MessageTypes.TaskApprovalRequired:
                return task.RequireApproval(envelope.ReadData<TaskApprovalRequiredData>().Reason, now);
            case MessageTypes.TaskCompleted:
                var completed = envelope.ReadData<TaskCompletedData>();
                var result = completed.Result is { ValueKind: JsonValueKind.Object } element ? JsonText.Compact(element) : "{}";
                return task.Complete(result, now);
            case MessageTypes.TaskFailed:
                return task.Fail(ToError(envelope.ReadData<TaskFailedData>().Error), now);
            case MessageTypes.TaskCancelled:
                var reason = envelope.ReadData<TaskCancelledData>().Reason;
                return task.ConfirmCancelled(string.IsNullOrWhiteSpace(reason) ? TaskErrorCodes.UserRequest : reason, now);
            default:
                throw new InvalidOperationException($"Unsupported task message {type}.");
        }
    }

    private static TaskError ToError(ErrorInfo? error) =>
        new(string.IsNullOrWhiteSpace(error?.Code) ? TaskErrorCodes.ExecutionError : error.Code, error?.Message);
}

/// <summary>HU-034: notifica al device origen cada cambio de estado de sus tareas (best effort).</summary>
public sealed class SourceDeviceTaskNotifier(
    IJarvisDbContext db,
    IRequestContext context,
    IDeviceMessagePublisher publisher,
    TimeProvider clock) : IEventSubscriber
{
    private static readonly HashSet<string> Notifiable =
    [
        EventTypes.TaskDispatched, EventTypes.TaskDelivered, EventTypes.TaskStarted, EventTypes.ApprovalRequested,
        EventTypes.TaskCompleted, EventTypes.TaskFailed, EventTypes.TaskCancelled, EventTypes.TaskExpired,
    ];

    public async Task HandleAsync(EventRecord record, CancellationToken cancellationToken)
    {
        if (record.TaskId is not { } taskId || !Notifiable.Contains(record.Type))
        {
            return;
        }

        context.EnterSystemScope();
        var task = await db.Tasks.AsNoTracking().FirstOrDefaultAsync(t => t.Id == taskId, cancellationToken);
        if (task?.SourceDeviceId is not { } sourceId)
        {
            return;
        }

        await publisher.PublishAsync(
            sourceId,
            MessageTypes.TaskUpdate,
            TaskMapper.ToUpdate(task, clock.GetUtcNow()),
            task.CorrelationId,
            null,
            cancellationToken);
    }
}

/// <summary>HU-027 / HU-032 / HU-034: mensajes <c>up/requests</c>.</summary>
public sealed class DeviceRequestHandler(
    IJarvisDbContext db,
    CreateTaskHandler createTask,
    CancelTaskHandler cancelTask,
    IDeviceMessagePublisher publisher,
    TimeProvider clock,
    ILogger<DeviceRequestHandler> logger)
{
    public async Task HandleAsync(Device device, MqttEnvelope envelope, bool rateLimited, CancellationToken cancellationToken)
    {
        var principal = new Principal(PrincipalType.Device, device.Id, device.OwnerId);
        var type = envelope.Type!;
        if (rateLimited)
        {
            await RejectAsync(device, envelope, type, ErrorCodes.RateLimited, "Too many task requests.", cancellationToken);
            return;
        }

        switch (type)
        {
            case MessageTypes.TaskCreate:
                var data = envelope.ReadData<TaskCreateData>();
                var result = await createTask.HandleAsync(
                    principal,
                    new CreateTaskCommand(
                        data.TargetDeviceId,
                        data.Type,
                        data.Payload,
                        data.ExpiresInSeconds,
                        data.MaxDurationSeconds,
                        envelope.CorrelationId,
                        data.IdempotencyKey,
                        "mqtt"),
                    cancellationToken);
                if (result.IsSuccess)
                {
                    var created = result.Value;
                    await publisher.PublishAsync(
                        device.Id,
                        MessageTypes.RequestAccepted,
                        new RequestAcceptedData(type, created.Task.Id, created.Task.Status.ToString(), created.Duplicate),
                        created.Task.CorrelationId,
                        envelope.Id,
                        cancellationToken);
                }
                else
                {
                    await RejectAsync(device, envelope, type, result.Error!.Code, result.Error.Message, cancellationToken);
                }

                break;

            case MessageTypes.TaskGet:
                var taskId = envelope.ReadData<TaskGetData>().TaskId;
                var task = await db.Tasks.AsNoTracking().FirstOrDefaultAsync(
                    t => t.Id == taskId && t.OwnerId == device.OwnerId && (t.SourceDeviceId == device.Id || t.TargetDeviceId == device.Id),
                    cancellationToken);
                if (task is null)
                {
                    await RejectAsync(device, envelope, type, ErrorCodes.NotFound, "Task not found.", cancellationToken);
                }
                else
                {
                    await publisher.PublishAsync(
                        device.Id,
                        MessageTypes.TaskUpdate,
                        TaskMapper.ToUpdate(task, clock.GetUtcNow()),
                        task.CorrelationId,
                        envelope.Id,
                        cancellationToken);
                }

                break;

            case MessageTypes.TaskCancelRequest:
                var cancelId = envelope.ReadData<TaskCancelRequestData>().TaskId;
                var cancel = await cancelTask.HandleAsync(principal, cancelId, cancellationToken);
                if (cancel.IsSuccess)
                {
                    await publisher.PublishAsync(
                        device.Id,
                        MessageTypes.RequestAccepted,
                        new RequestAcceptedData(type, cancel.Value.Task.Id, cancel.Value.Task.Status.ToString(), false),
                        cancel.Value.Task.CorrelationId,
                        envelope.Id,
                        cancellationToken);
                }
                else
                {
                    await RejectAsync(device, envelope, type, cancel.Error!.Code, cancel.Error.Message, cancellationToken);
                }

                break;
        }
    }

    public Task RejectAsync(Device device, MqttEnvelope envelope, string requestType, string code, string message, CancellationToken cancellationToken)
    {
        logger.LogInformation("Request {RequestType} from {DeviceId} rejected with {Code}", requestType, device.Id, code);
        return publisher.PublishAsync(
            device.Id,
            MessageTypes.RequestRejected,
            new RequestRejectedData(requestType, new ErrorInfo(code, message)),
            envelope.CorrelationId,
            envelope.Id,
            cancellationToken);
    }
}
