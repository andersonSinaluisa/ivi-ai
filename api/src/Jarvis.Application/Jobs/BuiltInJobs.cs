using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Sync;
using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Jobs;
using Jarvis.Domain.Memory;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Jobs;

// Handlers de jobs que solo usan la base de datos. Operan siempre sobre los datos del owner del job (el worker corre
// en modo sistema, sin filtro global): cada consulta filtra por OwnerId de forma explícita.

internal static class JobParameters
{
    public static int Int(JsonElement parameters, string name, int fallback) =>
        parameters.TryGetProperty(name, out var value) && value.TryGetInt32(out var n) ? n : fallback;
}

/// <summary>report.summary — solo lectura.</summary>
public sealed class ReportSummaryJob(IJarvisDbContext db, TimeProvider clock) : IJobHandler
{
    public string Type => JobCatalog.ReportSummary;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        var owner = context.Job.OwnerId;
        var hours = JobParameters.Int(context.Parameters, "hours", 24);
        var since = clock.GetUtcNow().AddHours(-hours);
        var memories = db.Memories.AsNoTracking().Where(m => m.OwnerId == owner);
        var result = new
        {
            hours,
            memories = await memories.CountAsync(m => m.DeletedAt == null, cancellationToken),
            memoriesChanged = await memories.CountAsync(m => m.UpdatedAt >= since, cancellationToken),
            memoriesDeleted = await memories.CountAsync(m => m.DeletedAt >= since, cancellationToken),
            relations = await db.MemoryRelations.AsNoTracking().CountAsync(r => r.OwnerId == owner && r.DeletedAt == null, cancellationToken),
            experiences = await db.Experiences.AsNoTracking().CountAsync(e => e.OwnerId == owner && e.DeletedAt == null, cancellationToken),
            skills = await db.Skills.AsNoTracking().CountAsync(s => s.OwnerId == owner && s.DeletedAt == null, cancellationToken),
            pendingProposals = await db.SkillProposals.AsNoTracking().CountAsync(p => p.OwnerId == owner && p.Status == ProposalStatuses.Pending, cancellationToken),
            openConflicts = await db.SyncConflicts.AsNoTracking().CountAsync(c => c.OwnerId == owner && c.Status == ConflictStatuses.Open, cancellationToken),
            pushes = await db.Events.AsNoTracking().CountAsync(e => e.OwnerId == owner && e.Type == EventTypes.SyncPushCompleted && e.RecordedAt >= since, cancellationToken),
            failedRuns = await db.JobRuns.AsNoTracking().CountAsync(r => r.OwnerId == owner && r.Status != RunStatuses.Succeeded && r.FinishedAt >= since, cancellationToken),
        };
        return new JobOutcome(result);
    }
}

/// <summary>
/// memory.consolidate — no borra nada. (1) Recuerdos con el mismo contenido exacto → relación DUPLICATE_OF sugerida
/// (sin verificar). (2) Experiencias verificadas y completadas con la misma firma y apoyo suficiente → propuesta de
/// habilidad (nunca una habilidad: la acepta el owner).
/// </summary>
public sealed class MemoryConsolidateJob(IJarvisDbContext db, IMemorySql sql, TimeProvider clock) : IJobHandler
{
    public string Type => JobCatalog.MemoryConsolidate;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        var owner = context.Job.OwnerId;
        var minSupport = JobParameters.Int(context.Parameters, "minSupport", 3);
        return await db.InTransactionAsync(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(owner, ct);
                var now = clock.GetUtcNow();

                // (1) Duplicados exactos: el más nuevo apunta al más antiguo.
                var hashed = await db.Memories.AsNoTracking()
                    .Where(m => m.OwnerId == owner && m.DeletedAt == null && m.ContentSha256 != null)
                    .Select(m => new { m.Id, m.ContentSha256, m.CreatedAt })
                    .ToListAsync(ct);
                var groups = hashed.GroupBy(m => Convert.ToHexString(m.ContentSha256!)).Where(g => g.Count() > 1).ToList();
                var existing = await db.MemoryRelations.AsNoTracking()
                    .Where(r => r.OwnerId == owner && r.Kind == "DUPLICATE_OF" && r.DeletedAt == null)
                    .Select(r => new { r.SourceId, r.TargetId })
                    .ToListAsync(ct);
                var linked = existing.Select(e => (e.SourceId, e.TargetId)).ToHashSet();
                var duplicates = new List<object>();
                foreach (var group in groups)
                {
                    var ordered = group.OrderBy(m => m.CreatedAt).ToList();
                    foreach (var newer in ordered.Skip(1))
                    {
                        if (linked.Contains((newer.Id, ordered[0].Id)))
                        {
                            continue;
                        }

                        db.MemoryRelations.Add(new MemoryRelation
                        {
                            Id = Guid.CreateVersion7(now),
                            OwnerId = owner,
                            SourceId = newer.Id,
                            TargetId = ordered[0].Id,
                            Kind = "DUPLICATE_OF",
                            Confidence = 0.9f,
                            Provenance = "consolidation",
                            Verified = false,
                            Note = "Mismo contenido exacto (consolidación)",
                            Version = 1,
                            Seq = await sql.NextSeqAsync(ct),
                            DeviceCreatedAt = now,
                            CreatedAt = now,
                            UpdatedAt = now,
                        });
                        duplicates.Add(new { memoryId = newer.Id, duplicateOf = ordered[0].Id });
                    }
                }

                // (2) Propuestas de habilidad.
                var experiences = await db.Experiences.AsNoTracking()
                    .Where(e => e.OwnerId == owner && e.DeletedAt == null && e.Verified && e.Completed && e.Signature != "")
                    .ToListAsync(ct);
                var proposals = await db.SkillProposals.Where(p => p.OwnerId == owner).ToDictionaryAsync(p => p.Signature, ct);
                var proposed = 0;
                foreach (var group in experiences.GroupBy(e => e.Signature))
                {
                    var support = group.Sum(e => e.Occurrences);
                    var latest = group.OrderByDescending(e => e.UpdatedAt).First();
                    if (support < minSupport || string.IsNullOrEmpty(latest.Procedure))
                    {
                        continue;
                    }

                    if (proposals.TryGetValue(group.Key, out var proposal))
                    {
                        if (proposal.Status == ProposalStatuses.Pending && proposal.SupportCount != support)
                        {
                            proposal.SupportCount = support;
                            proposal.ExperienceIds = group.Select(e => e.Id).ToArray();
                            proposal.UpdatedAt = now;
                        }

                        continue;
                    }

                    db.SkillProposals.Add(new SkillProposal
                    {
                        Id = Guid.CreateVersion7(now),
                        OwnerId = owner,
                        Signature = group.Key,
                        Title = latest.Goal.Length > 200 ? latest.Goal[..200] : latest.Goal,
                        Procedure = latest.Procedure!,
                        ExperienceIds = group.Select(e => e.Id).ToArray(),
                        SupportCount = support,
                        CreatedAt = now,
                        UpdatedAt = now,
                    });
                    db.AddEvent(new DomainEvent(
                        EventTypes.SkillProposed,
                        owner,
                        now,
                        Data: new Dictionary<string, object?> { ["signature"] = group.Key, ["support"] = support },
                        Actor: Actor.System));
                    proposed++;
                }

                await db.SaveChangesAsync(ct);
                return new JobOutcome(
                    new { duplicateRelations = duplicates.Count, skillProposals = proposed, experiencesConsidered = experiences.Count },
                    new { duplicates = duplicates.Take(50) });
            },
            cancellationToken);
    }
}

/// <summary>sync.compact — borra recibos de idempotencia más antiguos que receiptDays.</summary>
public sealed class SyncCompactJob(IJarvisDbContext db, TimeProvider clock) : IJobHandler
{
    public string Type => JobCatalog.SyncCompact;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        var days = JobParameters.Int(context.Parameters, "receiptDays", 30);
        var cutoff = clock.GetUtcNow().AddDays(-days);
        var deleted = await db.SyncReceipts
            .Where(r => r.OwnerId == context.Job.OwnerId && r.CreatedAt < cutoff)
            .ExecuteDeleteAsync(cancellationToken);
        return new JobOutcome(new { receiptsDeleted = deleted, olderThanDays = days });
    }
}

/// <summary>memory.purge_expired — sensible: requiere aprobación. Tombstone de los recuerdos con retención vencida.</summary>
public sealed class MemoryPurgeExpiredJob(IJarvisDbContext db, IMemorySql sql, TimeProvider clock) : IJobHandler
{
    public string Type => JobCatalog.MemoryPurgeExpired;

    public async Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken)
    {
        if (context.Job.ApprovalState != ApprovalStates.Approved)
        {
            throw new InvalidOperationException("memory.purge_expired requires approval.");
        }

        var owner = context.Job.OwnerId;
        return await db.InTransactionAsync(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(owner, ct);
                var now = clock.GetUtcNow();
                var candidates = await db.Memories
                    .Where(m => m.OwnerId == owner && m.DeletedAt == null && m.RetentionKind == "days")
                    .ToListAsync(ct);
                var expired = candidates.Where(m => m.IsExpired(now)).ToList();
                foreach (var memory in expired)
                {
                    await SyncPushHandler.TombstoneMemoryAsync(db, sql, memory, now, null, null, ct);
                }

                await db.SaveChangesAsync(ct);
                return new JobOutcome(new { tombstoned = expired.Count }, new { ids = expired.Select(m => m.Id).Take(200) });
            },
            cancellationToken);
    }
}
