namespace Jarvis.Domain.Memory;

// Memoria transferible (ADR-013). Registros sincronizados: versión por registro asignada por el Cloud y seq de
// cambios por owner (ADR-015). Los tiempos del device se guardan como dato informativo, nunca para decidir.

public static class MemoryKinds
{
    public static readonly IReadOnlyList<string> All = ["episodic", "semantic", "procedural", "experience", "entity"];
}

public static class Verifications
{
    public const string Unverified = "unverified";
    public const string Verified = "verified";
    public const string Disputed = "disputed";

    public static readonly IReadOnlyList<string> All = [Unverified, Verified, Disputed];
}

public static class RelationKinds
{
    public static readonly IReadOnlyList<string> All =
        ["BELONGS_TO", "DEPENDS_ON", "CAUSED_BY", "SOLVED_BY", "RELATED_TO", "LEARNED_FROM", "USED_IN", "DUPLICATE_OF"];
}

public static class Provenances
{
    public const string Inferred = "inferred";

    public static readonly IReadOnlyList<string> All = ["user", "agent", Inferred, "import", "consolidation"];
}

public static class SourceKinds
{
    public static readonly IReadOnlyList<string> All = ["user", "agent", "conversation", "tool", "import", "experience", "consolidation"];
}

/// <summary>Campos de un recuerdo que se versionan y fusionan.</summary>
public sealed record MemoryFields(
    string Kind,
    string Content,
    string Summary,
    IReadOnlyList<string> Tags,
    float Relevance,
    float Confidence,
    string Verification,
    string RetentionKind,
    int? RetentionDays)
{
    public static readonly IReadOnlyList<string> Names =
        ["kind", "content", "summary", "tags", "relevance", "confidence", "verification", "retention"];

    public bool Same(MemoryFields other, string field) => field switch
    {
        "kind" => Kind == other.Kind,
        "content" => Content == other.Content,
        "summary" => Summary == other.Summary,
        "tags" => Tags.Order(StringComparer.Ordinal).SequenceEqual(other.Tags.Order(StringComparer.Ordinal), StringComparer.Ordinal),
        "relevance" => Math.Abs(Relevance - other.Relevance) < 1e-6,
        "confidence" => Math.Abs(Confidence - other.Confidence) < 1e-6,
        "verification" => Verification == other.Verification,
        "retention" => RetentionKind == other.RetentionKind && RetentionDays == other.RetentionDays,
        _ => throw new ArgumentOutOfRangeException(nameof(field)),
    };

    public bool SameAs(MemoryFields other) => Names.All(n => Same(other, n));
}

public sealed record MergeOutcome(MemoryFields? Result, IReadOnlyList<string> ConflictingFields)
{
    public bool IsConflict => ConflictingFields.Count > 0;
}

/// <summary>
/// Fusión a tres vías por campo (ADR-015): lo que cambió solo un lado se toma de ese lado; lo que cambiaron ambos
/// con el mismo valor se acepta; lo que cambiaron ambos con valores distintos es conflicto. Las etiquetas se fusionan
/// como conjunto (añadidas por cualquiera, menos las quitadas por cualquiera) y nunca generan conflicto.
/// </summary>
public static class ThreeWayMerge
{
    public static MergeOutcome Merge(MemoryFields baseline, MemoryFields server, MemoryFields device)
    {
        var conflicts = new List<string>();
        T PickValue<T>(string field, T s, T d)
        {
            var serverChanged = !baseline.Same(server, field);
            var deviceChanged = !baseline.Same(device, field);
            if (serverChanged && deviceChanged && !server.Same(device, field))
            {
                conflicts.Add(field);
                return s;
            }

            return deviceChanged ? d : s;
        }

        var kind = PickValue("kind", server.Kind, device.Kind);
        var content = PickValue("content", server.Content, device.Content);
        var summary = PickValue("summary", server.Summary, device.Summary);
        var relevance = PickValue("relevance", server.Relevance, device.Relevance);
        var confidence = PickValue("confidence", server.Confidence, device.Confidence);
        var verification = PickValue("verification", server.Verification, device.Verification);
        var retention = PickValue("retention", (server.RetentionKind, server.RetentionDays), (device.RetentionKind, device.RetentionDays));
        var tags = MergeTags(baseline.Tags, server.Tags, device.Tags);

        if (conflicts.Count > 0)
        {
            return new MergeOutcome(null, conflicts);
        }

        return new MergeOutcome(
            new MemoryFields(kind, content, summary, tags, relevance, confidence, verification, retention.Item1, retention.Item2),
            []);
    }

    public static IReadOnlyList<string> MergeTags(IReadOnlyList<string> baseline, IReadOnlyList<string> server, IReadOnlyList<string> device)
    {
        var removed = baseline.Where(t => !server.Contains(t, StringComparer.Ordinal) || !device.Contains(t, StringComparer.Ordinal)).ToHashSet(StringComparer.Ordinal);
        return baseline.Concat(server).Concat(device)
            .Where(t => !removed.Contains(t))
            .Distinct(StringComparer.Ordinal)
            .Order(StringComparer.Ordinal)
            .ToList();
    }
}

public sealed class MemoryItem
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Kind { get; set; } = "semantic";

    /// <summary>Nulo en un tombstone (el contenido se purga al borrar).</summary>
    public string? Content { get; set; }

    public string Summary { get; set; } = string.Empty;

    public string[] Tags { get; set; } = [];

    public float Relevance { get; set; }

    public float Confidence { get; set; }

    public string Verification { get; set; } = Verifications.Unverified;

    public string RetentionKind { get; set; } = "permanent";

    public int? RetentionDays { get; set; }

    public string SourceKind { get; set; } = "user";

    public string? SourceReference { get; set; }

    public string SourceLabel { get; set; } = string.Empty;

    /// <summary>SHA-256 del contenido exacto en UTF-8 (valida que un vector corresponda a esta versión).</summary>
    public byte[]? ContentSha256 { get; set; }

    public Guid? OriginDeviceId { get; set; }

    public Guid? LastDeviceId { get; set; }

    public Guid? LastChangeId { get; set; }

    public int Version { get; set; }

    public long Seq { get; set; }

    public bool HasOpenConflict { get; set; }

    public DateTimeOffset DeviceCreatedAt { get; set; }

    public DateTimeOffset DeviceUpdatedAt { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public DateTimeOffset? DeletedAt { get; set; }

    public bool IsDeleted => DeletedAt is not null;

    public MemoryFields Fields() =>
        new(Kind, Content ?? string.Empty, Summary, Tags, Relevance, Confidence, Verification, RetentionKind, RetentionDays);

    public void Apply(MemoryFields fields)
    {
        Kind = fields.Kind;
        Content = fields.Content;
        Summary = fields.Summary;
        Tags = [.. fields.Tags];
        Relevance = fields.Relevance;
        Confidence = fields.Confidence;
        Verification = fields.Verification;
        RetentionKind = fields.RetentionKind;
        RetentionDays = fields.RetentionDays;
    }

    /// <summary>¿Venció por su política de retención (según la fecha de creación informada por el device)?</summary>
    public bool IsExpired(DateTimeOffset now) =>
        RetentionKind == "days" && RetentionDays is { } days && DeviceCreatedAt.AddDays(days) < now;

    /// <summary>Tombstone: se conserva la identidad y la versión; el contenido se purga.</summary>
    public void MarkDeleted(DateTimeOffset now)
    {
        DeletedAt = now;
        Content = null;
        Summary = string.Empty;
        Tags = [];
        SourceReference = null;
        ContentSha256 = null;
        HasOpenConflict = false;
    }
}

public sealed class MemoryVersion
{
    public Guid MemoryId { get; set; }

    public int Version { get; set; }

    public Guid OwnerId { get; set; }

    /// <summary>Snapshot (jsonb) de los datos en esa versión.</summary>
    public string Data { get; set; } = "{}";

    public Guid? ChangeId { get; set; }

    public Guid? DeviceId { get; set; }

    public DateTimeOffset RecordedAt { get; set; }
}

public sealed class MemoryRelation
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public Guid SourceId { get; set; }

    public Guid TargetId { get; set; }

    public string Kind { get; set; } = "RELATED_TO";

    public float Confidence { get; set; }

    public string Provenance { get; set; } = "user";

    public bool Verified { get; set; }

    public string? Note { get; set; }

    public Guid? OriginDeviceId { get; set; }

    public Guid? LastChangeId { get; set; }

    public int Version { get; set; }

    public long Seq { get; set; }

    public DateTimeOffset DeviceCreatedAt { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public DateTimeOffset? DeletedAt { get; set; }
}

public sealed class MemoryEmbedding
{
    public Guid MemoryId { get; set; }

    public string Model { get; set; } = string.Empty;

    public Guid OwnerId { get; set; }

    public int Dims { get; set; }

    public byte[] ContentSha256 { get; set; } = [];

    public float[] Vector { get; set; } = [];

    public Guid? LastChangeId { get; set; }

    public Guid? OriginDeviceId { get; set; }

    public int Version { get; set; }

    public long Seq { get; set; }

    public DateTimeOffset CreatedAt { get; set; }
}

public sealed class Experience
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Goal { get; set; } = string.Empty;

    public string Context { get; set; } = string.Empty;

    public string Plan { get; set; } = string.Empty;

    /// <summary>jsonb: pasos (herramienta, argumentos, resultado, verificación).</summary>
    public string Steps { get; set; } = "[]";

    public string[] Tools { get; set; } = [];

    public string Result { get; set; } = string.Empty;

    public string Evidence { get; set; } = "[]";

    public string Errors { get; set; } = "[]";

    public string? Procedure { get; set; }

    public bool Verified { get; set; }

    public bool Completed { get; set; }

    public int Occurrences { get; set; } = 1;

    public string Signature { get; set; } = string.Empty;

    public Guid? OriginDeviceId { get; set; }

    public Guid? LastChangeId { get; set; }

    public int Version { get; set; }

    public long Seq { get; set; }

    public DateTimeOffset DeviceCreatedAt { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public DateTimeOffset? DeletedAt { get; set; }
}

public static class SkillStatuses
{
    public const string Draft = "draft";
    public const string Verified = "verified";
    public const string Deprecated = "deprecated";

    public static readonly IReadOnlyList<string> All = [Draft, Verified, Deprecated];
}

public sealed class Skill
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Name { get; set; } = string.Empty;

    public string Description { get; set; } = string.Empty;

    public string Procedure { get; set; } = string.Empty;

    public int ProcedureVersion { get; set; } = 1;

    public string Status { get; set; } = SkillStatuses.Draft;

    /// <summary>jsonb: plataformas y herramientas requeridas.</summary>
    public string Compatibility { get; set; } = "{}";

    /// <summary>jsonb: riesgo máximo (R0–R4) de las herramientas que usa.</summary>
    public string Permissions { get; set; } = "{}";

    public string Source { get; set; } = "device";

    public Guid? ExperienceId { get; set; }

    public DateTimeOffset? VerifiedAt { get; set; }

    public Guid? VerifiedBy { get; set; }

    public Guid? OriginDeviceId { get; set; }

    public Guid? LastChangeId { get; set; }

    public int Version { get; set; }

    public long Seq { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public DateTimeOffset? DeletedAt { get; set; }
}

public sealed class SkillVersion
{
    public Guid SkillId { get; set; }

    public int Number { get; set; }

    public Guid OwnerId { get; set; }

    public string Procedure { get; set; } = string.Empty;

    public string? Changelog { get; set; }

    public string CreatedByType { get; set; } = "Device";

    public Guid? CreatedById { get; set; }

    public DateTimeOffset CreatedAt { get; set; }
}

public static class ProposalStatuses
{
    public const string Pending = "pending";
    public const string Accepted = "accepted";
    public const string Rejected = "rejected";
}

public sealed class SkillProposal
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Signature { get; set; } = string.Empty;

    public string Title { get; set; } = string.Empty;

    public string Procedure { get; set; } = string.Empty;

    public Guid[] ExperienceIds { get; set; } = [];

    public int SupportCount { get; set; }

    public string Status { get; set; } = ProposalStatuses.Pending;

    public Guid? SkillId { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public DateTimeOffset? DecidedAt { get; set; }
}

public sealed class SyncDevice
{
    public Guid DeviceId { get; set; }

    public Guid OwnerId { get; set; }

    public string[] Scopes { get; set; } = [];

    public DateTimeOffset? LastPushAt { get; set; }

    public DateTimeOffset? LastPullAt { get; set; }

    public long LastPullSeq { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }
}

/// <summary>Resultado guardado de cada changeId: hace idempotente el push.</summary>
public sealed class SyncReceipt
{
    public Guid OwnerId { get; set; }

    public Guid ChangeId { get; set; }

    public Guid DeviceId { get; set; }

    public string Response { get; set; } = "{}";

    public DateTimeOffset CreatedAt { get; set; }
}

public static class ConflictStatuses
{
    public const string Open = "open";
    public const string Resolved = "resolved";
}

public sealed class SyncConflict
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Entity { get; set; } = string.Empty;

    public Guid EntityId { get; set; }

    public string Kind { get; set; } = string.Empty;

    public string[] Fields { get; set; } = [];

    public int? BaseVersion { get; set; }

    public int ServerVersion { get; set; }

    public string? ServerData { get; set; }

    public string? DeviceData { get; set; }

    public Guid? DeviceId { get; set; }

    public Guid? ChangeId { get; set; }

    public string Status { get; set; } = ConflictStatuses.Open;

    public string? Resolution { get; set; }

    public DateTimeOffset? ResolvedAt { get; set; }

    public string? ResolvedByType { get; set; }

    public Guid? ResolvedById { get; set; }

    public DateTimeOffset CreatedAt { get; set; }
}
