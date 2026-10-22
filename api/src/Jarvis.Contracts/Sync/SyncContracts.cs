using System.Text.Json;

namespace Jarvis.Contracts.Sync;

// Protocolo jarvis.sync/1 (docs/03-protocols/sync-protocol.md, ejemplos normativos en sync-vectors.json).
// Los valores de enumeración viajan como strings en minúsculas: se modelan como constantes, no como enums.

public static class SyncProtocol
{
    public const string Version = "jarvis.sync/1";
    public const int MaxChangesPerPush = 200;
    public const int MaxPushBytes = 1024 * 1024;
    public const int DefaultPullLimit = 200;
    public const int MaxPullLimit = 500;
}

public static class SyncEntities
{
    public const string Memory = "memory";
    public const string Relation = "relation";
    public const string Embedding = "embedding";
    public const string Experience = "experience";
    public const string Skill = "skill";

    public static readonly IReadOnlyList<string> All = [Memory, Relation, Embedding, Experience, Skill];
}

public static class SyncOps
{
    public const string Upsert = "upsert";
    public const string Delete = "delete";
}

public static class SyncStatuses
{
    public const string Applied = "applied";
    public const string Merged = "merged";
    public const string Conflict = "conflict";
    public const string Rejected = "rejected";
}

public static class ConflictKinds
{
    public const string UpdateUpdate = "update_update";
    public const string UpdateDelete = "update_delete";
    public const string DeleteUpdate = "delete_update";
    public const string CreateCreate = "create_create";
}

public static class ConflictResolutions
{
    public const string KeepServer = "keep_server";
    public const string KeepDevice = "keep_device";
    public const string Custom = "custom";
}

public static class SyncScopes
{
    public const string MemoriesRead = "memories:read";
    public const string MemoriesWrite = "memories:write";
    public const string ExperiencesRead = "experiences:read";
    public const string ExperiencesWrite = "experiences:write";
    public const string SkillsRead = "skills:read";
    public const string SkillsWrite = "skills:write";

    public static readonly IReadOnlyList<string> All =
        [MemoriesRead, MemoriesWrite, ExperiencesRead, ExperiencesWrite, SkillsRead, SkillsWrite];

    /// <summary>Scope de lectura o escritura necesario para una entidad.</summary>
    public static string For(string entity, bool write) => entity switch
    {
        SyncEntities.Experience => write ? ExperiencesWrite : ExperiencesRead,
        SyncEntities.Skill => write ? SkillsWrite : SkillsRead,
        _ => write ? MemoriesWrite : MemoriesRead,
    };
}

// ---- Datos de cada entidad ------------------------------------------------------------

public sealed record RetentionData(string? Kind, int? Days = null);

public sealed record SourceData(string? Kind, string? Reference, string? Label);

public sealed record MemoryData(
    string? Kind,
    string? Content,
    string? Summary,
    IReadOnlyList<string>? Tags,
    float? Relevance,
    float? Confidence,
    string? Verification,
    RetentionData? Retention,
    SourceData? Source,
    DateTimeOffset? CreatedAt,
    DateTimeOffset? UpdatedAt);

public sealed record RelationData(
    Guid? SourceId,
    Guid? TargetId,
    string? Kind,
    float? Confidence,
    string? Provenance,
    bool? Verified,
    string? Note,
    DateTimeOffset? CreatedAt);

public sealed record EmbeddingData(string? Model, int? Dims, string? ContentHash, float[]? Vector);

public sealed record ExperienceStepData(string? Tool, JsonElement? Arguments, bool? Ok, string? Verification);

public sealed record ExperienceData(
    string? Goal,
    string? Context,
    string? Plan,
    IReadOnlyList<ExperienceStepData>? Steps,
    string? Result,
    IReadOnlyList<JsonElement>? Evidence,
    IReadOnlyList<string>? Errors,
    string? Procedure,
    bool? Verified,
    bool? Completed,
    int? Occurrences,
    string? Signature,
    DateTimeOffset? CreatedAt);

public sealed record SkillCompatibility(IReadOnlyList<string>? Platforms, IReadOnlyList<string>? RequiredTools);

public sealed record SkillPermissions(string? MaxRisk);

public sealed record SkillData(
    string? Name,
    string? Description,
    string? Procedure,
    string? Status,
    SkillCompatibility? Compatibility,
    SkillPermissions? Permissions,
    string? Source,
    Guid? ExperienceId,
    string? Changelog);

// ---- Push / pull ---------------------------------------------------------------------

public sealed record SyncChange(
    Guid? ChangeId,
    string? Entity,
    Guid? EntityId,
    string? Op,
    int? BaseVersion,
    JsonElement? Data);

public sealed record PushRequest(string? Protocol, IReadOnlyList<SyncChange>? Changes);

public sealed record PushResult
{
    public Guid ChangeId { get; init; }

    public string Status { get; init; } = SyncStatuses.Rejected;

    public string? Entity { get; init; }

    public Guid? EntityId { get; init; }

    public int? Version { get; init; }

    public long? Seq { get; init; }

    public JsonElement? Data { get; init; }

    public Guid? ConflictId { get; init; }

    public IReadOnlyList<string>? Fields { get; init; }

    public string? ConflictKind { get; init; }

    public string? Code { get; init; }

    public string? Message { get; init; }

    public bool Replayed { get; init; }
}

public sealed record PushResponse(IReadOnlyList<PushResult> Results, long Cursor);

public sealed record PullRequest(string? Protocol, long? Since, int? Limit, IReadOnlyList<string>? Entities, IReadOnlyList<string>? EmbeddingModels);

public sealed record PulledChange(
    long Seq,
    string Entity,
    Guid EntityId,
    string Op,
    int Version,
    Guid? OriginDeviceId,
    Guid? ChangeId,
    JsonElement? Data);

public sealed record PullResponse(IReadOnlyList<PulledChange> Changes, long NextSince, bool HasMore);

// ---- Conflictos ------------------------------------------------------------------------

public sealed record SyncConflictDto(
    Guid Id,
    string Entity,
    Guid EntityId,
    string ConflictKind,
    IReadOnlyList<string> Fields,
    int? BaseVersion,
    int ServerVersion,
    JsonElement? ServerData,
    JsonElement? DeviceData,
    Guid? DeviceId,
    string Status,
    DateTimeOffset CreatedAt,
    string? Resolution = null,
    DateTimeOffset? ResolvedAt = null);

public sealed record ResolveConflictRequest(string? Resolution, JsonElement? Data);

public sealed record ResolveConflictResponse(Guid ConflictId, string Status, int Version, long Seq, bool Deleted);

// ---- Devices y estado ----------------------------------------------------------------

public sealed record SetSyncScopesRequest(IReadOnlyList<string>? Scopes);

public sealed record SyncDeviceDto(Guid DeviceId, IReadOnlyList<string> Scopes, DateTimeOffset? LastPushAt, DateTimeOffset? LastPullAt, long LastPullSeq);

public sealed record SyncStatusResponse(
    Guid? DeviceId,
    IReadOnlyList<string> Scopes,
    DateTimeOffset? LastPushAt,
    DateTimeOffset? LastPullAt,
    long CurrentSeq,
    int OpenConflicts,
    int Memories);

// ---- Memoria -------------------------------------------------------------------------

public sealed record MemoryDto(
    Guid Id,
    int Version,
    string Kind,
    string Content,
    string Summary,
    IReadOnlyList<string> Tags,
    float Relevance,
    float Confidence,
    string Verification,
    RetentionData Retention,
    SourceData Source,
    Guid? OriginDeviceId,
    DateTimeOffset CreatedAt,
    DateTimeOffset UpdatedAt,
    bool HasOpenConflict);

public sealed record RelationDto(
    Guid Id,
    Guid SourceId,
    Guid TargetId,
    string Kind,
    float Confidence,
    string Provenance,
    bool Verified,
    string? Note = null);

public sealed record MemoryDetailResponse(MemoryDto Memory, IReadOnlyList<RelationDto> Relations, IReadOnlyList<string> EmbeddingModels);

public sealed record MemoryVersionDto(int Version, JsonElement? Data, Guid? DeviceId, DateTimeOffset RecordedAt);

public sealed record QueryEmbedding(string? Model, float[]? Vector);

public sealed record MemorySearchRequest(string? Query, QueryEmbedding? Embedding, int? Limit, float? MinConfidence, bool? ExpandGraph);

public sealed record SearchHitDto(
    Guid Id,
    int Version,
    string Kind,
    string Content,
    string Summary,
    IReadOnlyList<string> Tags,
    float Confidence,
    string Verification,
    double Score,
    string Via);

public sealed record MemorySearchResponse(string Method, IReadOnlyList<SearchHitDto> Memories, IReadOnlyList<RelationDto> Relations);

public sealed record MemoryExport(
    string Format,
    DateTimeOffset ExportedAt,
    Guid OwnerId,
    IReadOnlyList<ExportedRecord> Memories,
    IReadOnlyList<ExportedRecord> Relations,
    IReadOnlyList<ExportedRecord> Experiences,
    IReadOnlyList<ExportedRecord> Skills,
    string Sha256);

public sealed record ExportedRecord(Guid Id, int Version, JsonElement Data);

public sealed record HealthResponse(string Status, string ApiVersion, DateTimeOffset ServerTime, HealthFeatures Features);

public sealed record HealthFeatures(string Sync, string VectorSearch);
