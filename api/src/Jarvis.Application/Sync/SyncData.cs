using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Jarvis.Domain.Memory;

namespace Jarvis.Application.Sync;

/// <summary>Validación y conversión entre el formato de cable (jarvis.sync/1) y las entidades del esquema memory.</summary>
public static partial class SyncData
{
    public const int MaxContent = 8000;
    public const int MaxSummary = 500;
    public const int MaxTags = 32;
    public const int MaxTagLength = 64;

    public static string Hex(byte[] bytes) => Convert.ToHexStringLower(bytes);

    public static byte[] ContentHash(string content) => SHA256.HashData(Encoding.UTF8.GetBytes(content));

    public static T? Parse<T>(JsonElement? element)
        where T : class
    {
        if (element is not { ValueKind: JsonValueKind.Object } value)
        {
            return null;
        }

        try
        {
            return value.Deserialize<T>(JarvisJson.Options);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    public static JsonElement ToElement<T>(T value) => JsonSerializer.SerializeToElement(value, JarvisJson.Options);

    public static string ToJson<T>(T value) => JsonSerializer.Serialize(value, JarvisJson.Options);

    public static JsonElement? FromJson(string? json) => json is null ? null : JarvisJson.ParseElement(json);

    // ---- Recuerdos ------------------------------------------------------------------------

    public sealed record ValidMemory(MemoryFields Fields, SourceData Source, DateTimeOffset CreatedAt, DateTimeOffset UpdatedAt);

    public static Result<ValidMemory> ValidateMemory(MemoryData? data, DateTimeOffset now)
    {
        if (data is null)
        {
            return AppError.Validation("data", "Memory data is required.");
        }

        var errors = new Dictionary<string, string[]>();
        void Fail(string field, string message) => errors[field] = [message];

        var kind = data.Kind ?? string.Empty;
        if (!MemoryKinds.All.Contains(kind))
        {
            Fail("kind", "Unknown memory kind.");
        }

        var content = data.Content ?? string.Empty;
        if (content.Trim().Length == 0 || content.Length > MaxContent)
        {
            Fail("content", $"Content must have 1-{MaxContent} characters.");
        }

        var summary = data.Summary ?? string.Empty;
        if (summary.Length > MaxSummary)
        {
            Fail("summary", $"Summary must have at most {MaxSummary} characters.");
        }

        var tags = (data.Tags ?? []).Select(t => t.Trim()).Where(t => t.Length > 0).Distinct(StringComparer.Ordinal).ToList();
        if (tags.Count > MaxTags || tags.Any(t => t.Length > MaxTagLength))
        {
            Fail("tags", $"At most {MaxTags} tags of {MaxTagLength} characters.");
        }

        var relevance = data.Relevance ?? 0.5f;
        var confidence = data.Confidence ?? 0.8f;
        if (!InUnit(relevance) || !InUnit(confidence))
        {
            Fail("confidence", "Relevance and confidence must be between 0 and 1.");
        }

        var verification = data.Verification ?? Verifications.Unverified;
        if (!Verifications.All.Contains(verification))
        {
            Fail("verification", "Unknown verification state.");
        }

        var retentionKind = data.Retention?.Kind ?? "permanent";
        int? retentionDays = null;
        switch (retentionKind)
        {
            case "permanent":
                break;
            case "days" when data.Retention?.Days is >= 1 and <= 36500:
                retentionDays = data.Retention.Days;
                break;
            case "session":
                Fail("retention", "Session memories are device-local and are not synchronized.");
                break;
            default:
                Fail("retention", "Retention must be permanent or days (1-36500).");
                break;
        }

        var source = data.Source ?? new SourceData("device", null, null);
        if (source.Kind is null || !SourceKinds.All.Contains(source.Kind))
        {
            Fail("source", "Unknown source kind.");
        }

        if (errors.Count > 0)
        {
            return new AppError(ErrorCodes.ValidationFailed, "Validation failed.", errors);
        }

        if (SecretGuard.LooksLikeSecret(content) || SecretGuard.LooksLikeSecret(summary))
        {
            return new AppError(ErrorCodes.ContentRejected, "Content looks like a credential and is never stored in memory.");
        }

        var fields = new MemoryFields(kind, content, summary, tags, relevance, confidence, verification, retentionKind, retentionDays);
        var label = string.IsNullOrWhiteSpace(source.Label) ? source.Kind! : source.Label!.Trim();
        return new ValidMemory(
            fields,
            new SourceData(source.Kind, Truncate(source.Reference, 200), Truncate(label, 100)),
            data.CreatedAt ?? now,
            data.UpdatedAt ?? data.CreatedAt ?? now);
    }

    public static MemoryData ToData(MemoryItem m) => new(
        m.Kind,
        m.Content,
        m.Summary,
        m.Tags,
        m.Relevance,
        m.Confidence,
        m.Verification,
        m.RetentionKind == "days" ? new RetentionData("days", m.RetentionDays) : new RetentionData("permanent"),
        new SourceData(m.SourceKind, m.SourceReference, m.SourceLabel),
        m.DeviceCreatedAt,
        m.DeviceUpdatedAt);

    public static MemoryFields FieldsFromSnapshot(string json)
    {
        var data = JsonSerializer.Deserialize<MemoryData>(json, JarvisJson.Options)!;
        return new MemoryFields(
            data.Kind ?? "semantic",
            data.Content ?? string.Empty,
            data.Summary ?? string.Empty,
            data.Tags ?? [],
            data.Relevance ?? 0.5f,
            data.Confidence ?? 0.8f,
            data.Verification ?? Verifications.Unverified,
            data.Retention?.Kind ?? "permanent",
            data.Retention?.Kind == "days" ? data.Retention.Days : null);
    }

    public static MemoryDto ToDto(MemoryItem m) => new(
        m.Id,
        m.Version,
        m.Kind,
        m.Content ?? string.Empty,
        m.Summary,
        m.Tags,
        m.Relevance,
        m.Confidence,
        m.Verification,
        m.RetentionKind == "days" ? new RetentionData("days", m.RetentionDays) : new RetentionData("permanent"),
        new SourceData(m.SourceKind, m.SourceReference, m.SourceLabel),
        m.OriginDeviceId,
        m.CreatedAt,
        m.UpdatedAt,
        m.HasOpenConflict);

    // ---- Relaciones -----------------------------------------------------------------------

    public static Result<RelationData> ValidateRelation(RelationData? data)
    {
        if (data?.SourceId is null || data.TargetId is null)
        {
            return AppError.Validation("data", "sourceId and targetId are required.");
        }

        if (data.SourceId == data.TargetId)
        {
            return AppError.Validation("targetId", "A relation needs two distinct memories.");
        }

        if (data.Kind is null || !RelationKinds.All.Contains(data.Kind))
        {
            return AppError.Validation("kind", "Unknown relation kind.");
        }

        var provenance = data.Provenance ?? "user";
        if (!Provenances.All.Contains(provenance))
        {
            return AppError.Validation("provenance", "Unknown provenance.");
        }

        if (provenance == Provenances.Inferred && data.Verified == true)
        {
            return AppError.Validation("verified", "An inferred relation cannot be verified by itself.");
        }

        if (!InUnit(data.Confidence ?? 0.5f))
        {
            return AppError.Validation("confidence", "Confidence must be between 0 and 1.");
        }

        if (data.Note is { Length: > 500 })
        {
            return AppError.Validation("note", "Note must have at most 500 characters.");
        }

        return data with { Provenance = provenance, Confidence = data.Confidence ?? 0.5f, Verified = data.Verified ?? false };
    }

    public static RelationData ToData(MemoryRelation r) =>
        new(r.SourceId, r.TargetId, r.Kind, r.Confidence, r.Provenance, r.Verified, r.Note, r.DeviceCreatedAt);

    public static RelationDto ToDto(MemoryRelation r) =>
        new(r.Id, r.SourceId, r.TargetId, r.Kind, r.Confidence, r.Provenance, r.Verified, r.Note);

    public static bool SameRelation(MemoryRelation r, RelationData d) =>
        r.Kind == d.Kind
        && Math.Abs(r.Confidence - (d.Confidence ?? 0.5f)) < 1e-6
        && r.Provenance == d.Provenance
        && r.Verified == (d.Verified ?? false)
        && r.Note == d.Note;

    public static IReadOnlyList<string> RelationDiff(MemoryRelation r, RelationData d)
    {
        var fields = new List<string>();
        if (r.Kind != d.Kind)
        {
            fields.Add("kind");
        }

        if (Math.Abs(r.Confidence - (d.Confidence ?? 0.5f)) >= 1e-6)
        {
            fields.Add("confidence");
        }

        if (r.Provenance != d.Provenance)
        {
            fields.Add("provenance");
        }

        if (r.Verified != (d.Verified ?? false))
        {
            fields.Add("verified");
        }

        if (r.Note != d.Note)
        {
            fields.Add("note");
        }

        return fields;
    }

    // ---- Vectores ------------------------------------------------------------------------

    public static Result<(string Model, float[] Vector, byte[] Hash)> ValidateEmbedding(EmbeddingData? data)
    {
        if (data?.Model is not { Length: >= 1 and <= 128 } model)
        {
            return AppError.Validation("model", "Model name is required (1-128 characters).");
        }

        if (data.Vector is not { Length: >= 1 and <= 4096 } vector || data.Dims != vector.Length)
        {
            return AppError.Validation("vector", "dims must equal the vector length (1-4096).");
        }

        if (vector.Any(v => float.IsNaN(v) || float.IsInfinity(v)))
        {
            return AppError.Validation("vector", "Vector values must be finite.");
        }

        if (data.ContentHash is not { Length: 64 } hex || !hex.All(Uri.IsHexDigit))
        {
            return AppError.Validation("contentHash", "contentHash must be a SHA-256 in hex.");
        }

        return (model, vector, Convert.FromHexString(hex));
    }

    // ---- Experiencias --------------------------------------------------------------------

    public static Result<Experience> ValidateExperience(ExperienceData? data, DateTimeOffset now)
    {
        if (data?.Goal is not { } goal || goal.Trim().Length == 0 || goal.Length > 2000)
        {
            return AppError.Validation("goal", "Goal is required (1-2000 characters).");
        }

        var steps = data.Steps ?? [];
        if (steps.Count > 100)
        {
            return AppError.Validation("steps", "At most 100 steps.");
        }

        var texts = new[] { goal, data.Context ?? string.Empty, data.Plan ?? string.Empty, data.Result ?? string.Empty, data.Procedure ?? string.Empty };
        if (texts.Any(SecretGuard.LooksLikeSecret))
        {
            return new AppError(ErrorCodes.ContentRejected, "Experience looks like it contains a credential.");
        }

        if (data.Signature is { Length: > 128 })
        {
            return AppError.Validation("signature", "Signature must have at most 128 characters.");
        }

        var stepsJson = ToJson(steps);
        var evidenceJson = ToJson(data.Evidence ?? []);
        var errorsJson = ToJson(data.Errors ?? []);
        if (Encoding.UTF8.GetByteCount(stepsJson) + Encoding.UTF8.GetByteCount(evidenceJson) + Encoding.UTF8.GetByteCount(errorsJson) > 60_000)
        {
            return AppError.Validation("steps", "Experience is too large.");
        }

        return new Experience
        {
            Goal = goal,
            Context = Truncate(data.Context, 4000) ?? string.Empty,
            Plan = Truncate(data.Plan, 4000) ?? string.Empty,
            Steps = stepsJson,
            Tools = steps.Select(s => s.Tool ?? string.Empty).Where(t => t.Length > 0).Distinct(StringComparer.Ordinal).ToArray(),
            Result = Truncate(data.Result, 4000) ?? string.Empty,
            Evidence = evidenceJson,
            Errors = errorsJson,
            Procedure = Truncate(data.Procedure, 8000),
            Verified = data.Verified ?? false,
            Completed = data.Completed ?? false,
            Occurrences = Math.Max(1, data.Occurrences ?? 1),
            Signature = data.Signature ?? string.Empty,
            DeviceCreatedAt = data.CreatedAt ?? now,
        };
    }

    public static ExperienceData ToData(Experience e) => new(
        e.Goal,
        e.Context,
        e.Plan,
        JsonSerializer.Deserialize<List<ExperienceStepData>>(e.Steps, JarvisJson.Options),
        e.Result,
        JsonSerializer.Deserialize<List<JsonElement>>(e.Evidence, JarvisJson.Options),
        JsonSerializer.Deserialize<List<string>>(e.Errors, JarvisJson.Options),
        e.Procedure,
        e.Verified,
        e.Completed,
        e.Occurrences,
        e.Signature,
        e.DeviceCreatedAt);

    public static bool SameExperience(Experience a, Experience b) =>
        a.Goal == b.Goal && a.Context == b.Context && a.Plan == b.Plan && a.Steps == b.Steps && a.Result == b.Result
        && a.Evidence == b.Evidence && a.Errors == b.Errors && a.Procedure == b.Procedure && a.Verified == b.Verified
        && a.Completed == b.Completed && a.Occurrences == b.Occurrences && a.Signature == b.Signature;

    public static void CopyExperience(Experience from, Experience to)
    {
        to.Goal = from.Goal;
        to.Context = from.Context;
        to.Plan = from.Plan;
        to.Steps = from.Steps;
        to.Tools = from.Tools;
        to.Result = from.Result;
        to.Evidence = from.Evidence;
        to.Errors = from.Errors;
        to.Procedure = from.Procedure;
        to.Verified = from.Verified;
        to.Completed = from.Completed;
        to.Occurrences = from.Occurrences;
        to.Signature = from.Signature;
    }

    // ---- Habilidades ---------------------------------------------------------------------

    public static readonly IReadOnlyList<string> RiskLevels = ["R0", "R1", "R2", "R3", "R4"];

    public static Result<SkillData> ValidateSkill(SkillData? data)
    {
        if (data?.Name is not { } name || name.Trim().Length == 0 || name.Length > 200)
        {
            return AppError.Validation("name", "Name is required (1-200 characters).");
        }

        if (data.Procedure is null || data.Procedure.Length > 20000)
        {
            return AppError.Validation("procedure", "Procedure is required (at most 20000 characters).");
        }

        if (data.Permissions?.MaxRisk is { } risk && !RiskLevels.Contains(risk))
        {
            return AppError.Validation("permissions.maxRisk", "maxRisk must be R0-R4.");
        }

        if (SecretGuard.LooksLikeSecret(data.Procedure) || SecretGuard.LooksLikeSecret(data.Description ?? string.Empty))
        {
            return new AppError(ErrorCodes.ContentRejected, "Skill looks like it contains a credential.");
        }

        return data with
        {
            Description = Truncate(data.Description, 2000) ?? string.Empty,
            Compatibility = data.Compatibility ?? new SkillCompatibility(["windows"], []),
            Permissions = data.Permissions ?? new SkillPermissions("R1"),
            Source = data.Source ?? "device",
        };
    }

    public static SkillData ToData(Skill s) => new(
        s.Name,
        s.Description,
        s.Procedure,
        s.Status,
        JsonSerializer.Deserialize<SkillCompatibility>(s.Compatibility, JarvisJson.Options),
        JsonSerializer.Deserialize<SkillPermissions>(s.Permissions, JarvisJson.Options),
        s.Source,
        s.ExperienceId,
        null);

    // ---- Búsqueda ------------------------------------------------------------------------

    private static readonly HashSet<string> StopWords = new(StringComparer.Ordinal)
    {
        "el", "la", "los", "las", "un", "una", "unos", "unas", "de", "del", "al", "a", "en", "y", "o", "que", "qué",
        "es", "son", "se", "su", "sus", "lo", "le", "les", "por", "para", "con", "sin", "mi", "mis", "me", "tu", "tus",
        "como", "cómo", "cual", "cuál", "donde", "dónde", "cuando", "cuándo", "sabes", "hay", "usa", "usar",
        "the", "a", "an", "of", "to", "in", "and", "or", "is", "are", "what", "which", "my", "your", "do", "does",
    };

    /// <summary>Términos seguros para to_tsquery (solo letras y números, sin palabras vacías).</summary>
    public static IReadOnlyList<string> SearchTerms(string query) =>
        WordPattern().Matches(query.ToLowerInvariant())
            .Select(m => m.Value)
            .Where(w => w.Length >= 2 && !StopWords.Contains(w))
            .Distinct(StringComparer.Ordinal)
            .Take(16)
            .ToList();

    public static double Cosine(float[] a, float[] b)
    {
        if (a.Length != b.Length || a.Length == 0)
        {
            return 0;
        }

        double dot = 0, na = 0, nb = 0;
        for (var i = 0; i < a.Length; i++)
        {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }

        return na == 0 || nb == 0 ? 0 : dot / (Math.Sqrt(na) * Math.Sqrt(nb));
    }

    private static bool InUnit(float value) => value is >= 0 and <= 1 && !float.IsNaN(value);

    private static string? Truncate(string? value, int max) => value is null ? null : value.Length <= max ? value : value[..max];

    [GeneratedRegex(@"[\p{L}\p{N}]+")]
    private static partial Regex WordPattern();
}

/// <summary>
/// Segunda barrera contra credenciales en la memoria (la primera es el escritorio). Detecta formas evidentes; no
/// pretende ser exhaustiva.
/// </summary>
public static partial class SecretGuard
{
    public static bool LooksLikeSecret(string text) => !string.IsNullOrEmpty(text) && SecretPattern().IsMatch(text);

    [GeneratedRegex(
        @"-----BEGIN [A-Z ]*PRIVATE KEY-----|\b(sk-[A-Za-z0-9_\-]{20,}|ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9\-]{10,}|jdv_[A-Za-z0-9_\-]{20,})\b|\b(password|passwd|contraseña|api[_-]?key|client[_-]?secret|access[_-]?token)\b[""']?\s*[:=]\s*[""']?[^\s""',}\]]{6,}",
        RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex SecretPattern();
}
