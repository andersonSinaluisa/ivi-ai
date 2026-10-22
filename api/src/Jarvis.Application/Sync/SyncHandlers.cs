using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Memory;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Sync;

/// <summary>POST /api/v1/sync/pull: estado actual de los registros cambiados después del cursor, en orden de seq.</summary>
public sealed class SyncPullHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access, TimeProvider clock)
{
    public async Task<Result<PullResponse>> HandleAsync(Principal principal, PullRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsDevice)
        {
            return AppError.Forbidden("Only devices pull changes.");
        }

        if (request?.Protocol != SyncProtocol.Version)
        {
            return new AppError(ErrorCodes.UnsupportedProtocol, $"Expected protocol '{SyncProtocol.Version}'.");
        }

        var since = request.Since ?? 0;
        var limit = request.Limit ?? SyncProtocol.DefaultPullLimit;
        if (since < 0 || limit is < 1 or > SyncProtocol.MaxPullLimit)
        {
            return AppError.Validation("limit", $"since >= 0 and limit 1-{SyncProtocol.MaxPullLimit}.");
        }

        var scopes = await access.ScopesAsync(principal, cancellationToken);
        var allowed = SyncEntities.All.Where(e => scopes.Contains(SyncScopes.For(e, write: false))).ToList();
        var requested = request.Entities is { Count: > 0 } list ? list : SyncEntities.All;
        if (requested.Any(e => !SyncEntities.All.Contains(e)))
        {
            return AppError.Validation("entities", "Unknown entity.");
        }

        if (requested.Any(e => !allowed.Contains(e)))
        {
            return new AppError(ErrorCodes.SyncScopeMissing, "This device cannot read some of the requested entities.");
        }

        var models = (request.EmbeddingModels ?? []).Where(m => m.Length is > 0 and <= 128).Distinct(StringComparer.Ordinal).Take(4).ToList();
        var refs = await sql.ChangedSinceAsync(principal.OwnerId, since, limit, requested.ToList(), models, cancellationToken);
        var changes = await LoadAsync(refs, cancellationToken);

        var device = await db.SyncDevices.FirstOrDefaultAsync(d => d.DeviceId == principal.Id, cancellationToken);
        var nextSince = refs.Count == 0 ? since : refs[^1].Seq;
        if (device is not null)
        {
            device.LastPullAt = clock.GetUtcNow();
            device.LastPullSeq = Math.Max(device.LastPullSeq, nextSince);
            device.UpdatedAt = device.LastPullAt.Value;
            await db.SaveChangesAsync(cancellationToken);
        }

        return new PullResponse(changes, nextSince, refs.Count == limit);
    }

    private async Task<List<PulledChange>> LoadAsync(IReadOnlyList<ChangeRef> refs, CancellationToken ct)
    {
        Guid[] Ids(string entity) => refs.Where(r => r.Entity == entity).Select(r => r.EntityId).Distinct().ToArray();
        var memoryIds = Ids(SyncEntities.Memory);
        var relationIds = Ids(SyncEntities.Relation);
        var experienceIds = Ids(SyncEntities.Experience);
        var skillIds = Ids(SyncEntities.Skill);
        var memories = await db.Memories.AsNoTracking().Where(m => memoryIds.Contains(m.Id)).ToDictionaryAsync(m => m.Id, ct);
        var relations = await db.MemoryRelations.AsNoTracking().Where(r => relationIds.Contains(r.Id)).ToDictionaryAsync(r => r.Id, ct);
        var experiences = await db.Experiences.AsNoTracking().Where(e => experienceIds.Contains(e.Id)).ToDictionaryAsync(e => e.Id, ct);
        var skills = await db.Skills.AsNoTracking().Where(s => skillIds.Contains(s.Id)).ToDictionaryAsync(s => s.Id, ct);
        var embeddingIds = Ids(SyncEntities.Embedding);
        var embeddings = (await db.MemoryEmbeddings.AsNoTracking().Where(e => embeddingIds.Contains(e.MemoryId)).ToListAsync(ct))
            .ToDictionary(e => (e.MemoryId, e.Model));

        var result = new List<PulledChange>(refs.Count);
        foreach (var r in refs)
        {
            PulledChange? change = r.Entity switch
            {
                SyncEntities.Memory when memories.TryGetValue(r.EntityId, out var m) => new PulledChange(
                    r.Seq, r.Entity, m.Id, m.IsDeleted ? SyncOps.Delete : SyncOps.Upsert, m.Version, m.OriginDeviceId, m.LastChangeId,
                    m.IsDeleted ? null : SyncData.ToElement(SyncData.ToData(m))),
                SyncEntities.Relation when relations.TryGetValue(r.EntityId, out var rel) => new PulledChange(
                    r.Seq, r.Entity, rel.Id, rel.DeletedAt is null ? SyncOps.Upsert : SyncOps.Delete, rel.Version, rel.OriginDeviceId, rel.LastChangeId,
                    rel.DeletedAt is null ? SyncData.ToElement(SyncData.ToData(rel)) : null),
                SyncEntities.Experience when experiences.TryGetValue(r.EntityId, out var e) => new PulledChange(
                    r.Seq, r.Entity, e.Id, e.DeletedAt is null ? SyncOps.Upsert : SyncOps.Delete, e.Version, e.OriginDeviceId, e.LastChangeId,
                    e.DeletedAt is null ? SyncData.ToElement(SyncData.ToData(e)) : null),
                SyncEntities.Skill when skills.TryGetValue(r.EntityId, out var s) => new PulledChange(
                    r.Seq, r.Entity, s.Id, s.DeletedAt is null ? SyncOps.Upsert : SyncOps.Delete, s.Version, s.OriginDeviceId, s.LastChangeId,
                    s.DeletedAt is null ? SyncData.ToElement(SyncData.ToData(s)) : null),
                SyncEntities.Embedding when r.Model is not null && embeddings.TryGetValue((r.EntityId, r.Model), out var emb) => new PulledChange(
                    r.Seq, r.Entity, emb.MemoryId, SyncOps.Upsert, emb.Version, emb.OriginDeviceId, emb.LastChangeId,
                    SyncData.ToElement(new EmbeddingData(emb.Model, emb.Dims, SyncData.Hex(emb.ContentSha256), emb.Vector))),
                _ => null,
            };
            if (change is not null)
            {
                result.Add(change);
            }
        }

        return result;
    }
}

/// <summary>Permisos de sincronización por device (solo el owner) y estado de sincronización.</summary>
public sealed class SyncDeviceHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access, TimeProvider clock)
{
    public async Task<Result<SyncDeviceDto>> SetScopesAsync(Principal principal, Guid deviceId, SetSyncScopesRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Only the owner grants sync permissions.");
        }

        var scopes = (request?.Scopes ?? []).Distinct(StringComparer.Ordinal).ToArray();
        if (scopes.Any(s => !SyncScopes.All.Contains(s)))
        {
            return AppError.Validation("scopes", $"Valid scopes: {string.Join(", ", SyncScopes.All)}.");
        }

        var device = await db.Devices.AsNoTracking().FirstOrDefaultAsync(d => d.Id == deviceId, cancellationToken);
        if (device is null)
        {
            return AppError.NotFound("Device");
        }

        if (device.AdminState == DeviceAdminState.Revoked)
        {
            return new AppError(ErrorCodes.DeviceRevoked, "Device is revoked.");
        }

        var now = clock.GetUtcNow();
        var sync = await db.SyncDevices.FirstOrDefaultAsync(d => d.DeviceId == deviceId, cancellationToken);
        if (sync is null)
        {
            sync = new SyncDevice { DeviceId = deviceId, OwnerId = principal.OwnerId, CreatedAt = now };
            db.SyncDevices.Add(sync);
        }

        sync.Scopes = scopes;
        sync.UpdatedAt = now;
        db.AddEvent(new DomainEvent(
            EventTypes.SyncScopesChanged,
            principal.OwnerId,
            now,
            DeviceId: deviceId,
            Data: new Dictionary<string, object?> { ["scopes"] = scopes }));
        await db.SaveChangesAsync(cancellationToken);
        return ToDto(sync);
    }

    public async Task<Result<IReadOnlyList<SyncDeviceDto>>> ListAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var devices = await db.SyncDevices.AsNoTracking().OrderBy(d => d.CreatedAt).ToListAsync(cancellationToken);
        return devices.Select(ToDto).ToList();
    }

    public async Task<Result<SyncStatusResponse>> StatusAsync(Principal principal, CancellationToken cancellationToken)
    {
        var scopes = await access.ScopesAsync(principal, cancellationToken);
        var device = principal.IsDevice
            ? await db.SyncDevices.AsNoTracking().FirstOrDefaultAsync(d => d.DeviceId == principal.Id, cancellationToken)
            : null;
        var conflicts = await db.SyncConflicts.CountAsync(c => c.Status == ConflictStatuses.Open, cancellationToken);
        var memories = await db.Memories.CountAsync(m => m.DeletedAt == null, cancellationToken);
        var seq = await sql.CurrentSeqAsync(principal.OwnerId, cancellationToken);
        return new SyncStatusResponse(principal.IsDevice ? principal.Id : null, scopes, device?.LastPushAt, device?.LastPullAt, seq, conflicts, memories);
    }

    private static SyncDeviceDto ToDto(SyncDevice d) => new(d.DeviceId, d.Scopes, d.LastPushAt, d.LastPullAt, d.LastPullSeq);
}

/// <summary>Conflictos de sincronización: listarlos y resolverlos con una versión nueva (ADR-015 §5).</summary>
public sealed class SyncConflictHandler(IJarvisDbContext db, IMemorySql sql, SyncAccess access, TimeProvider clock)
{
    public async Task<Result<IReadOnlyList<SyncConflictDto>>> ListAsync(Principal principal, string? status, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesRead, cancellationToken) is { } denied)
        {
            return denied;
        }

        var wanted = status ?? ConflictStatuses.Open;
        var conflicts = await db.SyncConflicts.AsNoTracking()
            .Where(c => wanted == "all" || c.Status == wanted)
            .OrderByDescending(c => c.CreatedAt)
            .Take(200)
            .ToListAsync(cancellationToken);
        return conflicts.Select(ToDto).ToList();
    }

    public async Task<Result<ResolveConflictResponse>> ResolveAsync(Principal principal, Guid conflictId, ResolveConflictRequest? request, CancellationToken cancellationToken)
    {
        if (await access.RequireAsync(principal, SyncScopes.MemoriesWrite, cancellationToken) is { } denied)
        {
            return denied;
        }

        if (request?.Resolution is not (ConflictResolutions.KeepServer or ConflictResolutions.KeepDevice or ConflictResolutions.Custom))
        {
            return AppError.Validation("resolution", "resolution must be keep_server, keep_device or custom.");
        }

        return await db.InTransactionAsync<Result<ResolveConflictResponse>>(
            async ct =>
            {
                await sql.LockOwnerChangesAsync(principal.OwnerId, ct);
                var conflict = await db.SyncConflicts.FirstOrDefaultAsync(c => c.Id == conflictId, ct);
                if (conflict is null)
                {
                    return AppError.NotFound("Conflict");
                }

                if (conflict.Status != ConflictStatuses.Open)
                {
                    return new AppError(ErrorCodes.ConflictAlreadyResolved, "This conflict was already resolved.");
                }

                if (conflict.Entity != SyncEntities.Memory)
                {
                    return await ResolveGenericAsync(principal, conflict, request, ct);
                }

                var now = clock.GetUtcNow();
                var memory = await db.Memories.FirstOrDefaultAsync(m => m.Id == conflict.EntityId, ct);
                if (memory is null)
                {
                    return AppError.NotFound("Memory");
                }

                // Datos elegidos: null = borrado.
                JsonElement? chosen = request.Resolution switch
                {
                    ConflictResolutions.KeepServer => memory.IsDeleted ? null : SyncData.ToElement(SyncData.ToData(memory)),
                    ConflictResolutions.KeepDevice => SyncData.FromJson(conflict.DeviceData),
                    _ => request.Data,
                };
                if (request.Resolution == ConflictResolutions.Custom && chosen is null)
                {
                    return AppError.Validation("data", "custom requires data.");
                }

                long seq;
                if (chosen is null)
                {
                    seq = memory.IsDeleted
                        ? await BumpTombstoneAsync(memory, now, principal, ct)
                        : await SyncPushHandler.TombstoneMemoryAsync(db, sql, memory, now, principal.IsDevice ? principal.Id : null, null, ct);
                }
                else
                {
                    var valid = SyncData.ValidateMemory(SyncData.Parse<MemoryData>(chosen), now);
                    if (!valid.IsSuccess)
                    {
                        return valid.Error!;
                    }

                    memory.DeletedAt = null;
                    memory.Apply(valid.Value.Fields);
                    seq = await sql.NextSeqAsync(ct);
                    memory.Version += 1;
                    memory.Seq = seq;
                    memory.UpdatedAt = now;
                    memory.DeviceUpdatedAt = now;
                    memory.LastChangeId = null;
                    var hash = SyncData.ContentHash(memory.Content!);
                    if (memory.ContentSha256 is null || !memory.ContentSha256.AsSpan().SequenceEqual(hash))
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
                        DeviceId = principal.IsDevice ? principal.Id : null,
                        RecordedAt = now,
                    });
                }

                MarkResolved(principal, conflict, request.Resolution, now);
                var others = await db.SyncConflicts.AnyAsync(
                    c => c.EntityId == conflict.EntityId && c.Status == ConflictStatuses.Open && c.Id != conflict.Id, ct);
                memory.HasOpenConflict = others;
                await db.SaveChangesAsync(ct);
                return new ResolveConflictResponse(conflict.Id, ConflictStatuses.Resolved, memory.Version, seq, memory.IsDeleted);
            },
            cancellationToken);
    }

    /// <summary>Relaciones, experiencias y habilidades: sin fusión; se elige una de las dos versiones.</summary>
    private async Task<Result<ResolveConflictResponse>> ResolveGenericAsync(Principal principal, SyncConflict conflict, ResolveConflictRequest request, CancellationToken ct)
    {
        var now = clock.GetUtcNow();
        if (request.Resolution == ConflictResolutions.Custom)
        {
            return AppError.Validation("resolution", "custom is only available for memories; push a new change instead.");
        }

        var keepDevice = request.Resolution == ConflictResolutions.KeepDevice;
        var seq = await sql.NextSeqAsync(ct);
        int version;
        bool deleted;
        switch (conflict.Entity)
        {
            case SyncEntities.Relation:
            {
                var relation = await db.MemoryRelations.FirstOrDefaultAsync(r => r.Id == conflict.EntityId, ct);
                if (relation is null)
                {
                    return AppError.NotFound("Relation");
                }

                if (keepDevice)
                {
                    var data = SyncData.Parse<RelationData>(SyncData.FromJson(conflict.DeviceData));
                    if (data is null)
                    {
                        relation.DeletedAt = now;
                    }
                    else
                    {
                        relation.DeletedAt = null;
                        relation.Kind = data.Kind ?? relation.Kind;
                        relation.Confidence = data.Confidence ?? relation.Confidence;
                        relation.Provenance = data.Provenance ?? relation.Provenance;
                        relation.Verified = data.Verified ?? relation.Verified;
                        relation.Note = data.Note;
                    }
                }

                relation.Version += 1;
                relation.Seq = seq;
                relation.UpdatedAt = now;
                version = relation.Version;
                deleted = relation.DeletedAt is not null;
                break;
            }

            case SyncEntities.Experience:
            {
                var experience = await db.Experiences.FirstOrDefaultAsync(e => e.Id == conflict.EntityId, ct);
                if (experience is null)
                {
                    return AppError.NotFound("Experience");
                }

                if (keepDevice)
                {
                    var parsed = SyncData.Parse<ExperienceData>(SyncData.FromJson(conflict.DeviceData));
                    if (parsed is null)
                    {
                        experience.DeletedAt = now;
                    }
                    else if (SyncData.ValidateExperience(parsed, now) is { IsSuccess: true } valid)
                    {
                        experience.DeletedAt = null;
                        SyncData.CopyExperience(valid.Value, experience);
                    }
                }

                experience.Version += 1;
                experience.Seq = seq;
                experience.UpdatedAt = now;
                version = experience.Version;
                deleted = experience.DeletedAt is not null;
                break;
            }

            default:
            {
                var skill = await db.Skills.FirstOrDefaultAsync(s => s.Id == conflict.EntityId, ct);
                if (skill is null)
                {
                    return AppError.NotFound("Skill");
                }

                if (keepDevice)
                {
                    var parsed = SyncData.Parse<SkillData>(SyncData.FromJson(conflict.DeviceData));
                    if (parsed is null)
                    {
                        skill.DeletedAt = now;
                    }
                    else if (SyncData.ValidateSkill(parsed) is { IsSuccess: true } valid && valid.Value.Procedure != skill.Procedure)
                    {
                        skill.DeletedAt = null;
                        skill.Procedure = valid.Value.Procedure!;
                        skill.ProcedureVersion += 1;
                        skill.Status = SkillStatuses.Draft;
                        skill.VerifiedAt = null;
                        db.SkillVersions.Add(new SkillVersion
                        {
                            SkillId = skill.Id,
                            Number = skill.ProcedureVersion,
                            OwnerId = skill.OwnerId,
                            Procedure = skill.Procedure,
                            Changelog = "Resolución de conflicto",
                            CreatedByType = principal.IsUser ? "User" : "Device",
                            CreatedById = principal.Id,
                            CreatedAt = now,
                        });
                    }
                }

                skill.Version += 1;
                skill.Seq = seq;
                skill.UpdatedAt = now;
                version = skill.Version;
                deleted = skill.DeletedAt is not null;
                break;
            }
        }

        MarkResolved(principal, conflict, request.Resolution!, now);
        await db.SaveChangesAsync(ct);
        return new ResolveConflictResponse(conflict.Id, ConflictStatuses.Resolved, version, seq, deleted);
    }

    private async Task<long> BumpTombstoneAsync(MemoryItem memory, DateTimeOffset now, Principal principal, CancellationToken ct)
    {
        var seq = await sql.NextSeqAsync(ct);
        memory.Version += 1;
        memory.Seq = seq;
        memory.UpdatedAt = now;
        memory.LastDeviceId = principal.IsDevice ? principal.Id : null;
        return seq;
    }

    private void MarkResolved(Principal principal, SyncConflict conflict, string resolution, DateTimeOffset now)
    {
        conflict.Status = ConflictStatuses.Resolved;
        conflict.Resolution = resolution;
        conflict.ResolvedAt = now;
        conflict.ResolvedByType = principal.IsUser ? "User" : "Device";
        conflict.ResolvedById = principal.Id;
        db.AddEvent(new DomainEvent(
            EventTypes.SyncConflictResolved,
            principal.OwnerId,
            now,
            DeviceId: principal.IsDevice ? principal.Id : null,
            Data: new Dictionary<string, object?> { ["entity"] = conflict.Entity, ["entityId"] = conflict.EntityId, ["resolution"] = resolution }));
    }

    public static SyncConflictDto ToDto(SyncConflict c) => new(
        c.Id, c.Entity, c.EntityId, c.Kind, c.Fields, c.BaseVersion, c.ServerVersion,
        SyncData.FromJson(c.ServerData), SyncData.FromJson(c.DeviceData), c.DeviceId, c.Status, c.CreatedAt, c.Resolution, c.ResolvedAt);
}
