using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Events;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Events;

public sealed record EventListQuery(
    long? After,
    IReadOnlyList<string> Types,
    Guid? DeviceId,
    Guid? TaskId,
    Guid? CorrelationId,
    DateTimeOffset? From,
    DateTimeOffset? To,
    int? Limit);

/// <summary>HU-039: consulta del event log (solo users, solo su owner).</summary>
public sealed class EventQueryHandler(IJarvisDbContext db)
{
    public async Task<Result<EventsPage>> ListAsync(Principal principal, EventListQuery query, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var ownerId = principal.OwnerId;
        var events = db.Events.AsNoTracking().Where(e => e.OwnerId == ownerId);
        if (query.Types.Count > 0)
        {
            var types = query.Types.ToList();
            events = events.Where(e => types.Contains(e.Type));
        }

        if (query.DeviceId is { } deviceId)
        {
            events = events.Where(e => e.DeviceId == deviceId);
        }

        if (query.TaskId is { } taskId)
        {
            events = events.Where(e => e.TaskId == taskId);
        }

        if (query.CorrelationId is { } correlationId)
        {
            events = events.Where(e => e.CorrelationId == correlationId);
        }

        if (query.From is { } from)
        {
            events = events.Where(e => e.OccurredAt >= from);
        }

        if (query.To is { } to)
        {
            events = events.Where(e => e.OccurredAt <= to);
        }

        var limit = Limits.ClampLimit(query.Limit, 100, 500);
        List<EventRecord> items;
        if (query.After is { } after)
        {
            items = await events.Where(e => e.Seq > after).OrderBy(e => e.Seq).Take(limit + 1).ToListAsync(cancellationToken);
        }
        else
        {
            items = await events.OrderByDescending(e => e.Seq).Take(limit + 1).ToListAsync(cancellationToken);
        }

        var hasMore = items.Count > limit;
        var page = items.Take(limit).Select(ToDto).ToList();
        long? lastSeq = page.Count == 0 ? query.After : page.Max(e => e.Seq);
        return new EventsPage(page, lastSeq, hasMore);
    }

    public static EventDto ToDto(EventRecord record) =>
        new(
            record.Seq,
            record.Id,
            record.Type,
            record.OccurredAt,
            record.RecordedAt,
            record.CorrelationId,
            new ActorDto(record.ActorType.ToString(), record.ActorId),
            record.DeviceId,
            record.TaskId,
            JsonText.ToElement(record.Data));
}

/// <summary>HU-040: borrado por lotes de eventos y refresh tokens fuera de retención.</summary>
public sealed class RetentionService(IRetentionStore store, TimeProvider clock, IOptions<RetentionOptions> options, ILogger<RetentionService> logger)
{
    public async Task RunAsync(CancellationToken cancellationToken)
    {
        var settings = options.Value;
        var now = clock.GetUtcNow();
        var events = await DeleteInBatchesAsync(
            ct => store.DeleteEventsRecordedBeforeAsync(now.AddDays(-settings.EventsDays), settings.BatchSize, ct),
            settings.BatchSize,
            cancellationToken);
        var tokens = await DeleteInBatchesAsync(
            ct => store.DeleteRefreshTokensExpiredBeforeAsync(now.AddDays(-settings.RefreshTokensDays), settings.BatchSize, ct),
            settings.BatchSize,
            cancellationToken);
        logger.LogInformation("Retention deleted {Events} events and {Tokens} refresh tokens", events, tokens);
    }

    private static async Task<int> DeleteInBatchesAsync(Func<CancellationToken, Task<int>> batch, int batchSize, CancellationToken cancellationToken)
    {
        var total = 0;
        while (!cancellationToken.IsCancellationRequested)
        {
            var deleted = await batch(cancellationToken);
            total += deleted;
            if (deleted < batchSize)
            {
                break;
            }

            await Task.Delay(TimeSpan.FromMilliseconds(200), cancellationToken);
        }

        return total;
    }
}
