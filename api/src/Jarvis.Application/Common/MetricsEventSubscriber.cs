using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Common;

/// <summary>Deriva métricas de negocio de los eventos confirmados (un único punto de conteo).</summary>
public sealed class MetricsEventSubscriber(JarvisMetrics metrics, IJarvisDbContext db, IRequestContext context) : IEventSubscriber
{
    public async Task HandleAsync(EventRecord record, CancellationToken cancellationToken)
    {
        var data = JsonText.ToElement(record.Data);
        switch (record.Type)
        {
            case EventTypes.TaskCreated:
                metrics.TaskCreated(Read(data, "channel") ?? "unknown", record.ActorType.ToString());
                break;
            case EventTypes.TaskDispatched:
                metrics.DispatchAttempt(data.TryGetProperty("attempt", out var attempt) && attempt.TryGetInt32(out var n) ? n : 1);
                break;
            case EventTypes.TaskDelivered or EventTypes.TaskCompleted or EventTypes.TaskFailed or EventTypes.TaskCancelled or EventTypes.TaskExpired:
                await RecordTaskTimingsAsync(record, cancellationToken);
                break;
            case EventTypes.SecurityViolation:
                metrics.SecurityViolation(Read(data, "rule") ?? "unknown");
                break;
            case EventTypes.DeviceAuthFailed:
                metrics.AuthFailure("device");
                break;
            case EventTypes.UserLoginFailed:
                metrics.AuthFailure("user");
                break;
        }
    }

    private async Task RecordTaskTimingsAsync(EventRecord record, CancellationToken cancellationToken)
    {
        if (record.TaskId is not { } taskId)
        {
            return;
        }

        context.EnterSystemScope();
        var task = await db.Tasks.AsNoTracking().FirstOrDefaultAsync(t => t.Id == taskId, cancellationToken);
        if (task is null)
        {
            return;
        }

        if (record.Type == EventTypes.TaskDelivered)
        {
            if (task.DeliveredAt is { } delivered)
            {
                metrics.DeliveryLatency(delivered - task.CreatedAt);
            }

            return;
        }

        metrics.TaskTerminal(task.Status.ToString(), task.Status == JarvisTaskStatus.Completed ? null : task.ErrorCode);
        if (task is { StartedAt: { } started, CompletedAt: { } completed })
        {
            metrics.ExecutionDuration(completed - started, task.Status.ToString());
        }
    }

    private static string? Read(JsonElement data, string property) =>
        data.ValueKind == JsonValueKind.Object && data.TryGetProperty(property, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
}
