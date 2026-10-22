using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Memory;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;

namespace Jarvis.Application.Sync;

/// <summary>
/// POST /api/v1/sync/push (ADR-015). Cada cambio se aplica en su propia transacción, con el lock de cambios del
/// owner tomado (el orden de seq coincide con el de commit) y su resultado guardado por changeId (idempotencia).
/// </summary>
public sealed class SyncPushHandler(
    IJarvisDbContext db,
    IMemorySql sql,
    SyncAccess access,
    TimeProvider clock,
    ILogger<SyncPushHandler> logger)
{
    public async Task<Result<PushResponse>> HandleAsync(Principal principal, PushRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsDevice)
        {
            return AppError.Forbidden("Only devices push changes.");
        }

        if (request?.Protocol != SyncProtocol.Version)
        {
            return new AppError(ErrorCodes.UnsupportedProtocol, $"Expected protocol '{SyncProtocol.Version}'.");
        }

        var changes = request.Changes ?? [];
        if (changes.Count == 0 || changes.Count > SyncProtocol.MaxChangesPerPush)
        {
            return AppError.Validation("changes", $"Send between 1 and {SyncProtocol.MaxChangesPerPush} changes.");
        }

        var scopes = await access.ScopesAsync(principal, cancellationToken);
        var results = new List<PushResult>(changes.Count);
        foreach (var change in changes)
        {
            results.Add(await ApplyOneAsync(principal, scopes, change, cancellationToken));
        }

        var now = clock.GetUtcNow();
        var device = await db.SyncDevices.FirstOrDefaultAsync(d => d.DeviceId == principal.Id, cancellationToken);
        if (device is not null)
        {
            device.LastPushAt = now;
            device.UpdatedAt = now;
        }

        db.AddEvent(new DomainEvent(
            EventTypes.SyncPushCompleted,
            principal.OwnerId,
            now,
            DeviceId: principal.Id,
            Data: results.GroupBy(r => r.Status).ToDictionary(g => g.Key, g => (object?)g.Count())));
        await db.SaveChangesAsync(cancellationToken);

        var cursor = await sql.CurrentSeqAsync(principal.OwnerId, cancellationToken);
        return new PushResponse(results, cursor);
    }

    private async Task<PushResult> ApplyOneAsync(Principal principal, IReadOnlyList<string> scopes, SyncChange change, CancellationToken cancellationToken)
    {
        var changeId = change.ChangeId ?? Guid.Empty;
        PushResult Reject(string code, string message) =>
            new() { ChangeId = changeId, Status = SyncStatuses.Rejected, Entity = change.Entity, EntityId = change.EntityId, Code = code, Message = message };

        if (change.ChangeId is null || change.EntityId is null || change.EntityId == Guid.Empty)
        {
            return Reject(ErrorCodes.ValidationFailed, "changeId and entityId are required.");
        }

        if (change.Entity is null || !SyncEntities.All.Contains(change.Entity))
        {
            return Reject(ErrorCodes.ValidationFailed, "Unknown entity.");
        }

        if (change.Op is not (SyncOps.Upsert or SyncOps.Delete))
        {
            return Reject(ErrorCodes.ValidationFailed, "op must be upsert or delete.");
        }

        if (change.BaseVersion is < 0)
        {
            return Reject(ErrorCodes.ValidationFailed, "baseVersion must be null or >= 0.");
        }

        var scope = SyncScopes.For(change.Entity, write: true);
        if (!scopes.Contains(scope))
        {
            return Reject(ErrorCodes.SyncScopeMissing, $"Missing scope '{scope}'.");
        }

        try
        {
            return await db.InTransactionAsync(
                async ct =>
                {
                    await sql.LockOwnerChangesAsync(principal.OwnerId, ct);
                    var receipt = await db.SyncReceipts.AsNoTracking().FirstOrDefaultAsync(r => r.ChangeId == changeId, ct);
                    if (receipt is not null)
                    {
                        return JsonSerializer.Deserialize<PushResult>(receipt.Response, JarvisJson.Options)! with { Replayed = true };
                    }

                    var context = new ChangeContext(principal, change, changeId, clock.GetUtcNow());
                    var result = change.Entity switch
                    {
                        SyncEntities.Memory => await ApplyMemoryAsync(context, ct),
                        SyncEntities.Relation => await ApplyRelationAsync(context, ct),
                        SyncEntities.Embedding => await ApplyEmbeddingAsync(context, ct),
                        SyncEntities.Experience => await ApplyExperienceAsync(context, ct),
                        _ => await ApplySkillAsync(context, ct),
                    };

                    db.SyncReceipts.Add(new SyncReceipt
                    {
                        OwnerId = principal.OwnerId,
                        ChangeId = changeId,
                        DeviceId = principal.Id,
                        Response = SyncData.ToJson(result),
                        CreatedAt = context.Now,
                    });
                    await db.SaveChangesAsync(ct);
                    return result;
                },
                cancellationToken);
        }
        catch (DbUpdateException exception)
        {
            db.ResetTracking();
            logger.LogWarning(exception, "Sync change {ChangeId} ({Entity}) rejected by the database", changeId, change.Entity);
            return Reject(ErrorCodes.ValidationFailed, "The change violates a data constraint.");
        }
    }

    private sealed record ChangeContext(Principal Principal, SyncChange Change, Guid ChangeId, DateTimeOffset Now)
    {
        public Guid EntityId => Change.EntityId!.Value;

        public int? BaseVersion => Change.BaseVersion;

        public bool IsDelete => Change.Op == SyncOps.Delete;
    }

    private PushResult Applied(ChangeContext c, int version, long seq, JsonElement? data = null) =>
        new() { ChangeId = c.ChangeId, Status = data is null ? SyncStatuses.Applied : SyncStatuses.Merged, Entity = c.Change.Entity, EntityId = c.EntityId, Version = version, Seq = seq, Data = data };

    private static PushResult Rejected(ChangeContext c, AppError error) =>
        new() { ChangeId = c.ChangeId, Status = SyncStatuses.Rejected, Entity = c.Change.Entity, EntityId = c.EntityId, Code = error.Code, Message = error.Errors is null ? error.Message : $"{error.Message} {string.Join("; ", error.Errors.Select(e => $"{e.Key}: {string.Join(", ", e.Value)}"))}" };

    private PushResult Conflict(ChangeContext c, string kind, IReadOnlyList<string> fields, int serverVersion, JsonElement? serverData, JsonElement? deviceData)
    {
        var conflict = new SyncConflict
        {
            Id = Guid.CreateVersion7(c.Now),
            OwnerId = c.Principal.OwnerId,
            Entity = c.Change.Entity!,
            EntityId = c.EntityId,
            Kind = kind,
            Fields = [.. fields],
            BaseVersion = c.BaseVersion,
            ServerVersion = serverVersion,
            ServerData = serverData?.GetRawText(),
            DeviceData = deviceData?.GetRawText(),
            DeviceId = c.Principal.Id,
            ChangeId = c.ChangeId,
            CreatedAt = c.Now,
        };
        db.SyncConflicts.Add(conflict);
        db.AddEvent(new DomainEvent(
            EventTypes.SyncConflictDetected,
            c.Principal.OwnerId,
            c.Now,
            DeviceId: c.Principal.Id,
            Data: new Dictionary<string, object?> { ["entity"] = conflict.Entity, ["entityId"] = conflict.EntityId, ["kind"] = kind, ["fields"] = fields },
            Actor: Actor.Device(c.Principal.Id)));
        return new PushResult
        {
            ChangeId = c.ChangeId,
            Status = SyncStatuses.Conflict,
            Entity = c.Change.Entity,
            EntityId = c.EntityId,
            Version = serverVersion,
            ConflictId = conflict.Id,
            Fields = fields,
            ConflictKind = kind,
            Data = serverData,
        };
    }

    // ---- Recuerdos ------------------------------------------------------------------------

    private async Task<PushResult> ApplyMemoryAsync(ChangeContext c, CancellationToken ct)
    {
        var existing = await db.Memories.FirstOrDefaultAsync(m => m.Id == c.EntityId, ct);
        if (c.IsDelete)
        {
            return await DeleteMemoryAsync(c, existing, ct);
        }

        var valid = SyncData.ValidateMemory(SyncData.Parse<MemoryData>(c.Change.Data), c.Now);
        if (!valid.IsSuccess)
        {
            return Rejected(c, valid.Error!);
        }

        var incoming = valid.Value;
        var deviceData = c.Change.Data;
        if (existing is null)
        {
            // Nuevo (o el servidor no lo conoce, p. ej. tras restaurar un backup anterior): se crea.
            var memory = new MemoryItem
            {
                Id = c.EntityId,
                OwnerId = c.Principal.OwnerId,
                SourceKind = incoming.Source.Kind!,
                SourceReference = incoming.Source.Reference,
                SourceLabel = incoming.Source.Label ?? incoming.Source.Kind!,
                OriginDeviceId = c.Principal.Id,
                DeviceCreatedAt = incoming.CreatedAt,
                CreatedAt = c.Now,
            };
            memory.Apply(incoming.Fields);
            db.Memories.Add(memory);
            var seq = await CommitMemoryVersionAsync(c, memory, incoming, ct);
            return Applied(c, memory.Version, seq);
        }

        if (existing.IsDeleted)
        {
            return Conflict(c, ConflictKinds.UpdateDelete, [], existing.Version, null, deviceData);
        }

        var current = existing.Fields();
        if (c.BaseVersion is null)
        {
            return current.SameAs(incoming.Fields)
                ? Applied(c, existing.Version, existing.Seq)
                : await MemoryConflictAsync(c, existing, ConflictKinds.CreateCreate, Differences(current, incoming.Fields), deviceData);
        }

        if (c.BaseVersion > existing.Version)
        {
            return Rejected(c, AppError.Validation("baseVersion", "baseVersion is ahead of the server."));
        }

        if (c.BaseVersion == existing.Version)
        {
            if (current.SameAs(incoming.Fields))
            {
                return Applied(c, existing.Version, existing.Seq);
            }

            existing.Apply(incoming.Fields);
            var seq = await CommitMemoryVersionAsync(c, existing, incoming, ct);
            return Applied(c, existing.Version, seq);
        }

        // Cambios concurrentes: fusión a tres vías con la versión base del historial.
        var snapshot = await db.MemoryVersions.AsNoTracking()
            .FirstOrDefaultAsync(v => v.MemoryId == existing.Id && v.Version == c.BaseVersion, ct);
        if (snapshot is null)
        {
            var differences = Differences(current, incoming.Fields);
            return differences.Count == 0
                ? Applied(c, existing.Version, existing.Seq)
                : await MemoryConflictAsync(c, existing, ConflictKinds.UpdateUpdate, differences, deviceData);
        }

        var outcome = ThreeWayMerge.Merge(SyncData.FieldsFromSnapshot(snapshot.Data), current, incoming.Fields);
        if (outcome.IsConflict)
        {
            return await MemoryConflictAsync(c, existing, ConflictKinds.UpdateUpdate, outcome.ConflictingFields, deviceData);
        }

        if (!outcome.Result!.SameAs(current))
        {
            existing.Apply(outcome.Result);
            await CommitMemoryVersionAsync(c, existing, incoming with { Fields = outcome.Result }, ct);
        }

        return Applied(c, existing.Version, existing.Seq, SyncData.ToElement(SyncData.ToData(existing)));
    }

    private Task<PushResult> MemoryConflictAsync(ChangeContext c, MemoryItem existing, string kind, IReadOnlyList<string> fields, JsonElement? deviceData)
    {
        existing.HasOpenConflict = true;
        return Task.FromResult(Conflict(c, kind, fields, existing.Version, SyncData.ToElement(SyncData.ToData(existing)), deviceData));
    }

    private static List<string> Differences(MemoryFields a, MemoryFields b) =>
        MemoryFields.Names.Where(n => !a.Same(b, n)).ToList();

    /// <summary>Nueva versión: incrementa, asigna seq, guarda el snapshot y descarta vectores de contenido anterior.</summary>
    private async Task<long> CommitMemoryVersionAsync(ChangeContext c, MemoryItem memory, SyncData.ValidMemory incoming, CancellationToken ct)
    {
        var seq = await sql.NextSeqAsync(ct);
        memory.Version += 1;
        memory.Seq = seq;
        memory.UpdatedAt = c.Now;
        memory.DeviceUpdatedAt = incoming.UpdatedAt;
        memory.LastDeviceId = c.Principal.Id;
        memory.LastChangeId = c.ChangeId;
        var hash = SyncData.ContentHash(memory.Content!);
        if (memory.ContentSha256 is not null && !memory.ContentSha256.AsSpan().SequenceEqual(hash))
        {
            await db.MemoryEmbeddings.Where(e => e.MemoryId == memory.Id).ExecuteDeleteAsync(ct);
        }

        memory.ContentSha256 = hash;
        db.MemoryVersions.Add(new MemoryVersion
        {
            MemoryId = memory.Id,
            Version = memory.Version,
            OwnerId = memory.OwnerId,
            Data = SyncData.ToJson(SyncData.ToData(memory)),
            ChangeId = c.ChangeId,
            DeviceId = c.Principal.Id,
            RecordedAt = c.Now,
        });
        return seq;
    }

    private async Task<PushResult> DeleteMemoryAsync(ChangeContext c, MemoryItem? existing, CancellationToken ct)
    {
        if (existing is null)
        {
            // El servidor nunca lo tuvo: no hay nada que propagar.
            return Applied(c, 0, await sql.CurrentSeqAsync(c.Principal.OwnerId, ct));
        }

        if (existing.IsDeleted)
        {
            return Applied(c, existing.Version, existing.Seq);
        }

        if (c.BaseVersion != existing.Version)
        {
            existing.HasOpenConflict = true;
            return Conflict(c, ConflictKinds.DeleteUpdate, [], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), null);
        }

        var seq = await TombstoneMemoryAsync(existing, c.Now, c.Principal.Id, c.ChangeId, ct);
        return Applied(c, existing.Version, seq);
    }

    /// <summary>
    /// Tombstone (ADR-015): versión y seq nuevos, contenido e historial purgados, vectores borrados y las relaciones
    /// que lo usan también pasan a tombstone. Lo usan el push, la resolución de conflictos y la retención.
    /// </summary>
    internal static async Task<long> TombstoneMemoryAsync(IJarvisDbContext db, IMemorySql sql, MemoryItem memory, DateTimeOffset now, Guid? deviceId, Guid? changeId, CancellationToken ct)
    {
        var seq = await sql.NextSeqAsync(ct);
        memory.MarkDeleted(now);
        memory.Version += 1;
        memory.Seq = seq;
        memory.UpdatedAt = now;
        memory.LastDeviceId = deviceId;
        memory.LastChangeId = changeId;
        await db.MemoryVersions.Where(v => v.MemoryId == memory.Id).ExecuteDeleteAsync(ct);
        await db.MemoryEmbeddings.Where(e => e.MemoryId == memory.Id).ExecuteDeleteAsync(ct);
        var relations = await db.MemoryRelations
            .Where(r => (r.SourceId == memory.Id || r.TargetId == memory.Id) && r.DeletedAt == null)
            .ToListAsync(ct);
        foreach (var relation in relations)
        {
            relation.DeletedAt = now;
            relation.Version += 1;
            relation.Seq = await sql.NextSeqAsync(ct);
            relation.UpdatedAt = now;
            relation.LastChangeId = changeId;
        }

        return relations.Count == 0 ? seq : relations.Max(r => r.Seq);
    }

    private Task<long> TombstoneMemoryAsync(MemoryItem memory, DateTimeOffset now, Guid deviceId, Guid changeId, CancellationToken ct) =>
        TombstoneMemoryAsync(db, sql, memory, now, deviceId, changeId, ct);

    // ---- Relaciones -----------------------------------------------------------------------

    private async Task<PushResult> ApplyRelationAsync(ChangeContext c, CancellationToken ct)
    {
        var existing = await db.MemoryRelations.FirstOrDefaultAsync(r => r.Id == c.EntityId, ct);
        if (c.IsDelete)
        {
            if (existing is null)
            {
                return Applied(c, 0, await sql.CurrentSeqAsync(c.Principal.OwnerId, ct));
            }

            if (existing.DeletedAt is not null)
            {
                return Applied(c, existing.Version, existing.Seq);
            }

            if (c.BaseVersion != existing.Version)
            {
                return Conflict(c, ConflictKinds.DeleteUpdate, [], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), null);
            }

            existing.DeletedAt = c.Now;
            return Applied(c, ++existing.Version, existing.Seq = await NextAsync(existing, c, ct));
        }

        var valid = SyncData.ValidateRelation(SyncData.Parse<RelationData>(c.Change.Data));
        if (!valid.IsSuccess)
        {
            return Rejected(c, valid.Error!);
        }

        var data = valid.Value;
        if (existing is null)
        {
            var endpoints = await db.Memories.AsNoTracking()
                .Where(m => (m.Id == data.SourceId || m.Id == data.TargetId) && m.DeletedAt == null)
                .CountAsync(ct);
            if (endpoints != 2)
            {
                return Rejected(c, new AppError(ErrorCodes.ReferenceNotFound, "Both memories must exist (push them first)."));
            }

            var relation = new MemoryRelation
            {
                Id = c.EntityId,
                OwnerId = c.Principal.OwnerId,
                SourceId = data.SourceId!.Value,
                TargetId = data.TargetId!.Value,
                OriginDeviceId = c.Principal.Id,
                DeviceCreatedAt = data.CreatedAt ?? c.Now,
                CreatedAt = c.Now,
            };
            CopyRelation(data, relation);
            relation.Version = 1;
            relation.Seq = await NextAsync(relation, c, ct);
            db.MemoryRelations.Add(relation);
            return Applied(c, relation.Version, relation.Seq);
        }

        if (existing.SourceId != data.SourceId || existing.TargetId != data.TargetId)
        {
            return Rejected(c, AppError.Validation("sourceId", "The endpoints of a relation cannot change."));
        }

        if (existing.DeletedAt is not null)
        {
            return Conflict(c, ConflictKinds.UpdateDelete, [], existing.Version, null, c.Change.Data);
        }

        if (SyncData.SameRelation(existing, data))
        {
            return Applied(c, existing.Version, existing.Seq);
        }

        if (c.BaseVersion != existing.Version)
        {
            var kind = c.BaseVersion is null ? ConflictKinds.CreateCreate : ConflictKinds.UpdateUpdate;
            return Conflict(c, kind, SyncData.RelationDiff(existing, data), existing.Version, SyncData.ToElement(SyncData.ToData(existing)), c.Change.Data);
        }

        CopyRelation(data, existing);
        existing.Version += 1;
        existing.Seq = await NextAsync(existing, c, ct);
        return Applied(c, existing.Version, existing.Seq);
    }

    private async Task<long> NextAsync(MemoryRelation relation, ChangeContext c, CancellationToken ct)
    {
        relation.UpdatedAt = c.Now;
        relation.LastChangeId = c.ChangeId;
        return await sql.NextSeqAsync(ct);
    }

    private static void CopyRelation(RelationData data, MemoryRelation relation)
    {
        relation.Kind = data.Kind!;
        relation.Confidence = data.Confidence ?? 0.5f;
        relation.Provenance = data.Provenance ?? "user";
        relation.Verified = data.Verified ?? false;
        relation.Note = data.Note;
    }

    // ---- Vectores ------------------------------------------------------------------------

    private async Task<PushResult> ApplyEmbeddingAsync(ChangeContext c, CancellationToken ct)
    {
        if (c.IsDelete)
        {
            var model = c.Change.Data is { ValueKind: JsonValueKind.Object } d && d.TryGetProperty("model", out var m) ? m.GetString() : null;
            var query = db.MemoryEmbeddings.Where(e => e.MemoryId == c.EntityId);
            await (model is null ? query : query.Where(e => e.Model == model)).ExecuteDeleteAsync(ct);
            return Applied(c, 0, await sql.CurrentSeqAsync(c.Principal.OwnerId, ct));
        }

        var valid = SyncData.ValidateEmbedding(SyncData.Parse<EmbeddingData>(c.Change.Data));
        if (!valid.IsSuccess)
        {
            return Rejected(c, valid.Error!);
        }

        var (modelName, vector, hash) = valid.Value;
        var memory = await db.Memories.AsNoTracking().FirstOrDefaultAsync(x => x.Id == c.EntityId && x.DeletedAt == null, ct);
        if (memory is null)
        {
            return Rejected(c, new AppError(ErrorCodes.ReferenceNotFound, "The memory does not exist (push it first)."));
        }

        if (memory.ContentSha256 is null || !memory.ContentSha256.AsSpan().SequenceEqual(hash))
        {
            return Rejected(c, new AppError(ErrorCodes.StaleEmbedding, "The vector was computed from different content than the current version."));
        }

        var embedding = await db.MemoryEmbeddings.FirstOrDefaultAsync(e => e.MemoryId == c.EntityId && e.Model == modelName, ct);
        if (embedding is null)
        {
            embedding = new MemoryEmbedding { MemoryId = c.EntityId, Model = modelName, OwnerId = c.Principal.OwnerId, CreatedAt = c.Now };
            db.MemoryEmbeddings.Add(embedding);
        }

        embedding.Dims = vector.Length;
        embedding.Vector = vector;
        embedding.ContentSha256 = hash;
        embedding.OriginDeviceId = c.Principal.Id;
        embedding.LastChangeId = c.ChangeId;
        embedding.Version += 1;
        embedding.Seq = await sql.NextSeqAsync(ct);
        return Applied(c, embedding.Version, embedding.Seq);
    }

    // ---- Experiencias --------------------------------------------------------------------

    private async Task<PushResult> ApplyExperienceAsync(ChangeContext c, CancellationToken ct)
    {
        var existing = await db.Experiences.FirstOrDefaultAsync(e => e.Id == c.EntityId, ct);
        if (c.IsDelete)
        {
            if (existing is null)
            {
                return Applied(c, 0, await sql.CurrentSeqAsync(c.Principal.OwnerId, ct));
            }

            if (existing.DeletedAt is not null)
            {
                return Applied(c, existing.Version, existing.Seq);
            }

            if (c.BaseVersion != existing.Version)
            {
                return Conflict(c, ConflictKinds.DeleteUpdate, [], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), null);
            }

            existing.DeletedAt = c.Now;
            existing.Goal = string.Empty;
            existing.Context = string.Empty;
            existing.Plan = string.Empty;
            existing.Steps = "[]";
            existing.Evidence = "[]";
            existing.Errors = "[]";
            existing.Result = string.Empty;
            existing.Procedure = null;
            return await BumpExperienceAsync(c, existing, ct);
        }

        var valid = SyncData.ValidateExperience(SyncData.Parse<ExperienceData>(c.Change.Data), c.Now);
        if (!valid.IsSuccess)
        {
            return Rejected(c, valid.Error!);
        }

        var incoming = valid.Value;
        if (existing is null)
        {
            incoming.Id = c.EntityId;
            incoming.OwnerId = c.Principal.OwnerId;
            incoming.OriginDeviceId = c.Principal.Id;
            incoming.CreatedAt = c.Now;
            db.Experiences.Add(incoming);
            return await BumpExperienceAsync(c, incoming, ct);
        }

        if (existing.DeletedAt is not null)
        {
            return Conflict(c, ConflictKinds.UpdateDelete, [], existing.Version, null, c.Change.Data);
        }

        if (SyncData.SameExperience(existing, incoming))
        {
            return Applied(c, existing.Version, existing.Seq);
        }

        if (c.BaseVersion != existing.Version)
        {
            var kind = c.BaseVersion is null ? ConflictKinds.CreateCreate : ConflictKinds.UpdateUpdate;
            return Conflict(c, kind, ["experience"], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), c.Change.Data);
        }

        SyncData.CopyExperience(incoming, existing);
        return await BumpExperienceAsync(c, existing, ct);
    }

    private async Task<PushResult> BumpExperienceAsync(ChangeContext c, Experience experience, CancellationToken ct)
    {
        experience.Version += 1;
        experience.Seq = await sql.NextSeqAsync(ct);
        experience.UpdatedAt = c.Now;
        experience.LastChangeId = c.ChangeId;
        return Applied(c, experience.Version, experience.Seq);
    }

    // ---- Habilidades ---------------------------------------------------------------------

    private async Task<PushResult> ApplySkillAsync(ChangeContext c, CancellationToken ct)
    {
        var existing = await db.Skills.FirstOrDefaultAsync(s => s.Id == c.EntityId, ct);
        if (c.IsDelete)
        {
            if (existing is null)
            {
                return Applied(c, 0, await sql.CurrentSeqAsync(c.Principal.OwnerId, ct));
            }

            if (existing.DeletedAt is not null)
            {
                return Applied(c, existing.Version, existing.Seq);
            }

            if (c.BaseVersion != existing.Version)
            {
                return Conflict(c, ConflictKinds.DeleteUpdate, [], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), null);
            }

            existing.DeletedAt = c.Now;
            return await BumpSkillAsync(c, existing, ct);
        }

        var valid = SyncData.ValidateSkill(SyncData.Parse<SkillData>(c.Change.Data));
        if (!valid.IsSuccess)
        {
            return Rejected(c, valid.Error!);
        }

        var data = valid.Value;
        if (existing is null)
        {
            // Un device solo propone: la habilidad nace en borrador aunque diga otra cosa (verificar es del user).
            var skill = new Skill
            {
                Id = c.EntityId,
                OwnerId = c.Principal.OwnerId,
                OriginDeviceId = c.Principal.Id,
                CreatedAt = c.Now,
                Status = SkillStatuses.Draft,
                ExperienceId = data.ExperienceId,
                Source = data.Source ?? "device",
            };
            CopySkill(data, skill);
            db.Skills.Add(skill);
            AddSkillVersion(skill, data.Changelog, c);
            var result = await BumpSkillAsync(c, skill, ct);
            return data.Status == SkillStatuses.Draft ? result : result with { Status = SyncStatuses.Merged, Data = SyncData.ToElement(SyncData.ToData(skill)) };
        }

        if (existing.DeletedAt is not null)
        {
            return Conflict(c, ConflictKinds.UpdateDelete, [], existing.Version, null, c.Change.Data);
        }

        var sameContent = existing.Name == data.Name && existing.Description == data.Description && existing.Procedure == data.Procedure
            && existing.Compatibility == SyncData.ToJson(data.Compatibility) && existing.Permissions == SyncData.ToJson(data.Permissions);
        if (sameContent)
        {
            return Applied(c, existing.Version, existing.Seq);
        }

        if (c.BaseVersion != existing.Version)
        {
            var kind = c.BaseVersion is null ? ConflictKinds.CreateCreate : ConflictKinds.UpdateUpdate;
            return Conflict(c, kind, ["skill"], existing.Version, SyncData.ToElement(SyncData.ToData(existing)), c.Change.Data);
        }

        var procedureChanged = existing.Procedure != data.Procedure;
        CopySkill(data, existing);
        if (procedureChanged)
        {
            // Un procedimiento distinto es otra habilidad hasta que el user la vuelva a verificar.
            existing.ProcedureVersion += 1;
            existing.Status = SkillStatuses.Draft;
            existing.VerifiedAt = null;
            existing.VerifiedBy = null;
            AddSkillVersion(existing, data.Changelog, c);
        }

        var updated = await BumpSkillAsync(c, existing, ct);
        return data.Status == existing.Status ? updated : updated with { Status = SyncStatuses.Merged, Data = SyncData.ToElement(SyncData.ToData(existing)) };
    }

    private static void CopySkill(SkillData data, Skill skill)
    {
        skill.Name = data.Name!.Trim();
        skill.Description = data.Description ?? string.Empty;
        skill.Procedure = data.Procedure!;
        skill.Compatibility = SyncData.ToJson(data.Compatibility);
        skill.Permissions = SyncData.ToJson(data.Permissions);
    }

    private void AddSkillVersion(Skill skill, string? changelog, ChangeContext c) =>
        db.SkillVersions.Add(new SkillVersion
        {
            SkillId = skill.Id,
            Number = skill.ProcedureVersion,
            OwnerId = skill.OwnerId,
            Procedure = skill.Procedure,
            Changelog = changelog,
            CreatedByType = "Device",
            CreatedById = c.Principal.Id,
            CreatedAt = c.Now,
        });

    private async Task<PushResult> BumpSkillAsync(ChangeContext c, Skill skill, CancellationToken ct)
    {
        skill.Version += 1;
        skill.Seq = await sql.NextSeqAsync(ct);
        skill.UpdatedAt = c.Now;
        skill.LastChangeId = c.ChangeId;
        return Applied(c, skill.Version, skill.Seq);
    }
}
