using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Memory;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Sync;

/// <summary>Consulta y recuperación de recuerdos (sync-protocol.md §6).</summary>
public sealed class MemoryQueryHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access, TimeProvider clock)
{
    public const int MaxApplicationVectorScan = 50_000;

    public async Task<Result<PagedMemories>> ListAsync(Principal principal, string? kind, string? q, int? limit, string? cursor, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var take = Math.Clamp(limit ?? 50, 1, 200);
        var query = db.Memories.AsNoTracking().Where(m => m.DeletedAt == null);
        if (!string.IsNullOrEmpty(kind))
        {
            query = query.Where(m => m.Kind == kind);
        }

        if (!string.IsNullOrWhiteSpace(q))
        {
            var ids = (await sql.TextSearchAsync(principal.OwnerId, SyncData.SearchTerms(q), 1000, cancellationToken)).Select(h => h.Id).ToList();
            query = query.Where(m => ids.Contains(m.Id));
        }

        if (KeysetCursor.TryDecode(cursor, out var after, out var afterId))
        {
            query = query.Where(m => m.UpdatedAt < after || (m.UpdatedAt == after && m.Id.CompareTo(afterId) < 0));
        }

        var items = await query.OrderByDescending(m => m.UpdatedAt).ThenByDescending(m => m.Id).Take(take + 1).ToListAsync(cancellationToken);
        var page = items.Take(take).Select(SyncData.ToDto).ToList();
        var next = items.Count > take ? KeysetCursor.Encode(items[take - 1].UpdatedAt, items[take - 1].Id) : null;
        return new PagedMemories(page, next);
    }

    public async Task<Result<MemoryDetailResponse>> GetAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var memory = await db.Memories.AsNoTracking().FirstOrDefaultAsync(m => m.Id == id && m.DeletedAt == null, cancellationToken);
        if (memory is null)
        {
            return AppError.NotFound("Memory");
        }

        var relations = await db.MemoryRelations.AsNoTracking()
            .Where(r => (r.SourceId == id || r.TargetId == id) && r.DeletedAt == null)
            .ToListAsync(cancellationToken);
        var models = await db.MemoryEmbeddings.AsNoTracking().Where(e => e.MemoryId == id).Select(e => e.Model).ToListAsync(cancellationToken);
        return new MemoryDetailResponse(SyncData.ToDto(memory), relations.Select(SyncData.ToDto).ToList(), models);
    }

    public async Task<Result<IReadOnlyList<MemoryVersionDto>>> VersionsAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        if (!await db.Memories.AnyAsync(m => m.Id == id, cancellationToken))
        {
            return AppError.NotFound("Memory");
        }

        var versions = await db.MemoryVersions.AsNoTracking()
            .Where(v => v.MemoryId == id)
            .OrderByDescending(v => v.Version)
            .Take(100)
            .ToListAsync(cancellationToken);
        return versions.Select(v => new MemoryVersionDto(v.Version, SyncData.FromJson(v.Data), v.DeviceId, v.RecordedAt)).ToList();
    }

    /// <summary>
    /// Recuperación (ADR-014): vector del device contra vectores del mismo modelo (pgvector o en la aplicación),
    /// combinado con búsqueda de texto; filtros de confianza, verificación y retención; expansión por el grafo.
    /// </summary>
    public async Task<Result<MemorySearchResponse>> SearchAsync(Principal principal, MemorySearchRequest? request, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        if (request?.Query is not { } query || query.Trim().Length == 0 || query.Length > 1000)
        {
            return AppError.Validation("query", "query is required (1-1000 characters).");
        }

        var limit = Math.Clamp(request.Limit ?? 8, 1, 50);
        var minConfidence = request.MinConfidence ?? 0.3f;
        var candidates = new Dictionary<Guid, (double Vector, double Text)>();
        var method = "text";

        if (request.Embedding is { Model: { Length: > 0 } model, Vector: { Length: >= 1 and <= 4096 } vector })
        {
            IReadOnlyList<ScoredId> vectorHits;
            if (await sql.VectorExtensionAvailableAsync(cancellationToken))
            {
                vectorHits = await sql.VectorSearchAsync(principal.OwnerId, model, vector, limit * 4, cancellationToken);
                method = "vector:pgvector";
            }
            else
            {
                vectorHits = await ApplicationVectorSearchAsync(model, vector, limit * 4, cancellationToken);
                method = "vector:application";
            }

            if (vectorHits.Count == 0)
            {
                method = "text";
            }

            foreach (var hit in vectorHits)
            {
                candidates[hit.Id] = (hit.Score, 0);
            }
        }

        var textHits = await sql.TextSearchAsync(principal.OwnerId, SyncData.SearchTerms(query), limit * 4, cancellationToken);
        var maxText = textHits.Count == 0 ? 1 : Math.Max(textHits.Max(h => h.Score), 1e-9);
        foreach (var hit in textHits)
        {
            var normalized = hit.Score / maxText;
            candidates[hit.Id] = candidates.TryGetValue(hit.Id, out var c) ? (c.Vector, normalized) : (0, normalized);
        }

        var useVector = method != "text";
        var now = clock.GetUtcNow();
        var ids = candidates.Keys.ToList();
        var memories = await db.Memories.AsNoTracking().Where(m => ids.Contains(m.Id) && m.DeletedAt == null).ToListAsync(cancellationToken);
        var hits = memories
            .Where(m => m.Verification != Verifications.Disputed && m.Confidence >= minConfidence && !m.IsExpired(now))
            .Select(m =>
            {
                var (v, t) = candidates[m.Id];
                var score = useVector ? (0.8 * v) + (0.2 * t) : t;
                score *= 0.85 + (0.15 * m.Confidence);
                return (Memory: m, Score: score);
            })
            .OrderByDescending(x => x.Score)
            .Take(limit)
            .ToList();

        var result = hits.Select(h => ToHit(h.Memory, h.Score, "match")).ToList();
        var selected = hits.Select(h => h.Memory.Id).ToHashSet();
        if ((request.ExpandGraph ?? true) && selected.Count > 0)
        {
            var selectedIds = selected.ToList();
            var neighbors = await db.MemoryRelations.AsNoTracking()
                .Where(r => r.DeletedAt == null && (r.Verified || r.Provenance == "user")
                    && (selectedIds.Contains(r.SourceId) || selectedIds.Contains(r.TargetId)))
                .Take(200)
                .ToListAsync(cancellationToken);
            var neighborIds = neighbors.SelectMany(r => new[] { r.SourceId, r.TargetId }).Where(id => !selected.Contains(id)).Distinct().ToList();
            var extra = await db.Memories.AsNoTracking()
                .Where(m => neighborIds.Contains(m.Id) && m.DeletedAt == null && m.Verification != Verifications.Disputed)
                .ToListAsync(cancellationToken);
            var scoreOf = hits.ToDictionary(h => h.Memory.Id, h => h.Score);
            foreach (var memory in extra.Where(m => !m.IsExpired(now) && m.Confidence >= minConfidence))
            {
                var best = neighbors
                    .Where(r => r.SourceId == memory.Id || r.TargetId == memory.Id)
                    .Select(r => scoreOf.GetValueOrDefault(r.SourceId == memory.Id ? r.TargetId : r.SourceId) * 0.5 * r.Confidence)
                    .DefaultIfEmpty(0)
                    .Max();
                result.Add(ToHit(memory, best, "graph"));
            }

            result = result.Where(h => h.Via == "match").Concat(result.Where(h => h.Via == "graph").OrderByDescending(h => h.Score).Take(Math.Max(2, limit / 2))).ToList();
        }

        var returned = result.Select(h => h.Id).ToList();
        var relations = await db.MemoryRelations.AsNoTracking()
            .Where(r => r.DeletedAt == null && returned.Contains(r.SourceId) && returned.Contains(r.TargetId))
            .ToListAsync(cancellationToken);
        return new MemorySearchResponse(method, result, relations.Select(SyncData.ToDto).ToList());
    }

    private async Task<IReadOnlyList<ScoredId>> ApplicationVectorSearchAsync(string model, float[] vector, int limit, CancellationToken cancellationToken)
    {
        var rows = await db.MemoryEmbeddings.AsNoTracking()
            .Where(e => e.Model == model && e.Dims == vector.Length)
            .Join(db.Memories.Where(m => m.DeletedAt == null), e => e.MemoryId, m => m.Id, (e, m) => new { e.MemoryId, e.Vector, e.ContentSha256, Current = m.ContentSha256 })
            .Take(MaxApplicationVectorScan)
            .ToListAsync(cancellationToken);
        return rows
            .Where(r => r.Current is not null && r.Current.AsSpan().SequenceEqual(r.ContentSha256))
            .Select(r => new ScoredId(r.MemoryId, SyncData.Cosine(vector, r.Vector)))
            .OrderByDescending(s => s.Score)
            .Take(limit)
            .ToList();
    }

    private static SearchHitDto ToHit(MemoryItem m, double score, string via) =>
        new(m.Id, m.Version, m.Kind, m.Content ?? string.Empty, m.Summary, m.Tags, m.Confidence, m.Verification, Math.Round(score, 4), via);

    /// <summary>Exportación portable (jarvis.memory-export/1) con SHA-256 del contenido para validarla al importar.</summary>
    public async Task<Result<MemoryExport>> ExportAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Only the owner exports memory.");
        }

        var memories = await db.Memories.AsNoTracking().Where(m => m.DeletedAt == null).OrderBy(m => m.CreatedAt).ToListAsync(cancellationToken);
        var relations = await db.MemoryRelations.AsNoTracking().Where(r => r.DeletedAt == null).OrderBy(r => r.CreatedAt).ToListAsync(cancellationToken);
        var experiences = await db.Experiences.AsNoTracking().Where(e => e.DeletedAt == null).OrderBy(e => e.CreatedAt).ToListAsync(cancellationToken);
        var skills = await db.Skills.AsNoTracking().Where(s => s.DeletedAt == null).OrderBy(s => s.CreatedAt).ToListAsync(cancellationToken);
        var m = memories.Select(x => new ExportedRecord(x.Id, x.Version, SyncData.ToElement(SyncData.ToData(x)))).ToList();
        var r = relations.Select(x => new ExportedRecord(x.Id, x.Version, SyncData.ToElement(SyncData.ToData(x)))).ToList();
        var e = experiences.Select(x => new ExportedRecord(x.Id, x.Version, SyncData.ToElement(SyncData.ToData(x)))).ToList();
        var s = skills.Select(x => new ExportedRecord(x.Id, x.Version, SyncData.ToElement(SyncData.ToData(x)))).ToList();
        return new MemoryExport("jarvis.memory-export/1", clock.GetUtcNow(), principal.OwnerId, m, r, e, s, ExportHash(m, r, e, s));
    }

    public static string ExportHash(params IReadOnlyList<ExportedRecord>[] sections)
    {
        var json = JsonSerializer.Serialize(sections, JarvisJson.Options);
        return Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(json)));
    }
}

public sealed record PagedMemories(IReadOnlyList<MemoryDto> Items, string? NextCursor);

/// <summary>Experiencias: consulta y recuperación de soluciones anteriores.</summary>
public sealed class ExperienceQueryHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access)
{
    public async Task<Result<IReadOnlyList<ExportedRecord>>> ListAsync(Principal principal, string? signature, int? limit, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.ExperiencesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var query = db.Experiences.AsNoTracking().Where(e => e.DeletedAt == null);
        if (!string.IsNullOrEmpty(signature))
        {
            query = query.Where(e => e.Signature == signature);
        }

        var items = await query.OrderByDescending(e => e.UpdatedAt).Take(Math.Clamp(limit ?? 50, 1, 200)).ToListAsync(cancellationToken);
        return items.Select(e => new ExportedRecord(e.Id, e.Version, SyncData.ToElement(SyncData.ToData(e)))).ToList();
    }

    /// <summary>Soluciones anteriores para un objetivo: verificadas y completadas primero.</summary>
    public async Task<Result<IReadOnlyList<ExportedRecord>>> SimilarAsync(Principal principal, string? goal, int? limit, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.ExperiencesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        if (string.IsNullOrWhiteSpace(goal) || goal.Length > 1000)
        {
            return AppError.Validation("goal", "goal is required (1-1000 characters).");
        }

        var take = Math.Clamp(limit ?? 5, 1, 20);
        var hits = await sql.ExperienceSearchAsync(principal.OwnerId, SyncData.SearchTerms(goal), take * 3, cancellationToken);
        var ids = hits.Select(h => h.Id).ToList();
        var experiences = await db.Experiences.AsNoTracking().Where(e => ids.Contains(e.Id)).ToListAsync(cancellationToken);
        var score = hits.ToDictionary(h => h.Id, h => h.Score);
        return experiences
            .OrderByDescending(e => e.Verified && e.Completed)
            .ThenByDescending(e => score[e.Id])
            .ThenByDescending(e => e.Occurrences)
            .Take(take)
            .Select(e => new ExportedRecord(e.Id, e.Version, SyncData.ToElement(SyncData.ToData(e))))
            .ToList();
    }
}

public sealed record SkillDto(
    Guid Id,
    int Version,
    string Name,
    string Description,
    string Procedure,
    int ProcedureVersion,
    string Status,
    JsonElement Compatibility,
    JsonElement Permissions,
    string Source,
    Guid? ExperienceId,
    DateTimeOffset? VerifiedAt,
    DateTimeOffset UpdatedAt);

public sealed record SkillVersionDto(int Number, string Procedure, string? Changelog, string CreatedByType, DateTimeOffset CreatedAt);

public sealed record SkillProposalDto(Guid Id, string Title, string Procedure, IReadOnlyList<Guid> ExperienceIds, int SupportCount, string Status, Guid? SkillId, DateTimeOffset CreatedAt);

public sealed record CreateSkillRequest(string? Name, string? Description, string? Procedure, SkillCompatibility? Compatibility, SkillPermissions? Permissions);

public sealed record UpdateSkillStatusRequest(string? Status);

/// <summary>Skill Registry: catálogo, versiones, verificación (solo user) y propuestas. Nada se ejecuta aquí.</summary>
public sealed class SkillHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access, TimeProvider clock)
{
    public async Task<Result<IReadOnlyList<SkillDto>>> ListAsync(Principal principal, string? status, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.SkillsRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var query = db.Skills.AsNoTracking().Where(s => s.DeletedAt == null);
        if (!string.IsNullOrEmpty(status))
        {
            query = query.Where(s => s.Status == status);
        }

        return (await query.OrderBy(s => s.Name).Take(500).ToListAsync(cancellationToken)).Select(ToDto).ToList();
    }

    public async Task<Result<SkillDto>> GetAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.SkillsRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var skill = await db.Skills.AsNoTracking().FirstOrDefaultAsync(s => s.Id == id && s.DeletedAt == null, cancellationToken);
        return skill is null ? AppError.NotFound("Skill") : ToDto(skill);
    }

    public async Task<Result<IReadOnlyList<SkillVersionDto>>> VersionsAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.SkillsRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var versions = await db.SkillVersions.AsNoTracking().Where(v => v.SkillId == id).OrderByDescending(v => v.Number).ToListAsync(cancellationToken);
        return versions.Count == 0
            ? AppError.NotFound("Skill")
            : versions.Select(v => new SkillVersionDto(v.Number, v.Procedure, v.Changelog, v.CreatedByType, v.CreatedAt)).ToList();
    }

    public async Task<Result<SkillDto>> CreateAsync(Principal principal, CreateSkillRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Devices propose skills through sync.");
        }

        var valid = SyncData.ValidateSkill(new SkillData(request?.Name, request?.Description, request?.Procedure, SkillStatuses.Draft, request?.Compatibility, request?.Permissions, "user", null, null));
        if (!valid.IsSuccess)
        {
            return valid.Error!;
        }

        return await db.InTransactionAsync<Result<SkillDto>>(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(principal.OwnerId, ct);
                var skill = await NewSkillAsync(principal, valid.Value, null, "Creada por el usuario", ct);
                await db.SaveChangesAsync(ct);
                return ToDto(skill);
            },
            cancellationToken);
    }

    /// <summary>Verificar o retirar una habilidad: acto explícito del owner (los devices la reciben desactivada igual).</summary>
    public async Task<Result<SkillDto>> SetStatusAsync(Principal principal, Guid id, UpdateSkillStatusRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Only the owner verifies skills.");
        }

        if (request?.Status is not { } status || !SkillStatuses.All.Contains(status))
        {
            return AppError.Validation("status", "status must be draft, verified or deprecated.");
        }

        return await db.InTransactionAsync<Result<SkillDto>>(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(principal.OwnerId, ct);
                var skill = await db.Skills.FirstOrDefaultAsync(s => s.Id == id && s.DeletedAt == null, ct);
                if (skill is null)
                {
                    return AppError.NotFound("Skill");
                }

                if (skill.Status == status)
                {
                    return ToDto(skill);
                }

                var now = clock.GetUtcNow();
                skill.Status = status;
                skill.VerifiedAt = status == SkillStatuses.Verified ? now : null;
                skill.VerifiedBy = status == SkillStatuses.Verified ? principal.Id : null;
                skill.Version += 1;
                skill.Seq = await sql.NextSeqAsync(ct);
                skill.UpdatedAt = now;
                skill.LastChangeId = null;
                db.AddEvent(new DomainEvent(
                    EventTypes.SkillStatusChanged,
                    principal.OwnerId,
                    now,
                    Data: new Dictionary<string, object?> { ["skillId"] = skill.Id, ["status"] = status }));
                await db.SaveChangesAsync(ct);
                return ToDto(skill);
            },
            cancellationToken);
    }

    public async Task<Result<IReadOnlyList<SkillProposalDto>>> ProposalsAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var proposals = await db.SkillProposals.AsNoTracking().OrderByDescending(p => p.UpdatedAt).Take(200).ToListAsync(cancellationToken);
        return proposals.Select(ToDto).ToList();
    }

    public async Task<Result<SkillProposalDto>> DecideProposalAsync(Principal principal, Guid id, bool accept, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        return await db.InTransactionAsync<Result<SkillProposalDto>>(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(principal.OwnerId, ct);
                var proposal = await db.SkillProposals.FirstOrDefaultAsync(p => p.Id == id, ct);
                if (proposal is null)
                {
                    return AppError.NotFound("Proposal");
                }

                if (proposal.Status != ProposalStatuses.Pending)
                {
                    return new AppError(ErrorCodes.InvalidStateTransition, "The proposal was already decided.");
                }

                var now = clock.GetUtcNow();
                proposal.Status = accept ? ProposalStatuses.Accepted : ProposalStatuses.Rejected;
                proposal.DecidedAt = now;
                proposal.UpdatedAt = now;
                if (accept)
                {
                    var data = new SkillData(proposal.Title, $"Propuesta a partir de {proposal.SupportCount} experiencias verificadas", proposal.Procedure, SkillStatuses.Draft, new SkillCompatibility(["windows"], []), new SkillPermissions("R1"), "proposal", proposal.ExperienceIds.FirstOrDefault(), null);
                    var valid = SyncData.ValidateSkill(data);
                    if (!valid.IsSuccess)
                    {
                        return valid.Error!;
                    }

                    var skill = await NewSkillAsync(principal, valid.Value, proposal.ExperienceIds.FirstOrDefault(), "Aceptada desde una propuesta", ct);
                    proposal.SkillId = skill.Id;
                }

                await db.SaveChangesAsync(ct);
                return ToDto(proposal);
            },
            cancellationToken);
    }

    private async Task<Skill> NewSkillAsync(Principal principal, SkillData data, Guid? experienceId, string changelog, CancellationToken ct)
    {
        var now = clock.GetUtcNow();
        var skill = new Skill
        {
            Id = Guid.CreateVersion7(now),
            OwnerId = principal.OwnerId,
            Name = data.Name!.Trim(),
            Description = data.Description ?? string.Empty,
            Procedure = data.Procedure!,
            Compatibility = SyncData.ToJson(data.Compatibility),
            Permissions = SyncData.ToJson(data.Permissions),
            Source = data.Source ?? "user",
            ExperienceId = experienceId,
            Status = SkillStatuses.Draft,
            Version = 1,
            Seq = await sql.NextSeqAsync(ct),
            CreatedAt = now,
            UpdatedAt = now,
        };
        db.Skills.Add(skill);
        db.SkillVersions.Add(new SkillVersion
        {
            SkillId = skill.Id,
            Number = 1,
            OwnerId = skill.OwnerId,
            Procedure = skill.Procedure,
            Changelog = changelog,
            CreatedByType = "User",
            CreatedById = principal.Id,
            CreatedAt = now,
        });
        return skill;
    }

    public static SkillDto ToDto(Skill s) => new(
        s.Id, s.Version, s.Name, s.Description, s.Procedure, s.ProcedureVersion, s.Status,
        JarvisJson.ParseElement(s.Compatibility), JarvisJson.ParseElement(s.Permissions), s.Source, s.ExperienceId, s.VerifiedAt, s.UpdatedAt);

    public static SkillProposalDto ToDto(SkillProposal p) =>
        new(p.Id, p.Title, p.Procedure, p.ExperienceIds, p.SupportCount, p.Status, p.SkillId, p.CreatedAt);
}
