using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Jarvis.Application.Sync;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Jarvis.Domain.Memory;

namespace Jarvis.Application.Tests;

/// <summary>
/// jarvis.sync/1: los vectores normativos de docs/03-protocols/sync-vectors.json se leen con los contratos del Cloud.
/// El escritorio (Rust) lee el mismo archivo en sus pruebas.
/// </summary>
public sealed class SyncContractTests
{
    private static readonly JsonElement Vectors = JarvisJson.ParseElement(
        File.ReadAllText(Path.Combine(RepoPaths.Root, "docs", "03-protocols", "sync-vectors.json")));

    private static T Read<T>(string name) => Vectors.GetProperty(name).Deserialize<T>(JarvisJson.Options)!;

    /// <summary>Compara JSON ignorando nulos (el Cloud no escribe propiedades nulas) y el orden de propiedades.</summary>
    private static void AssertSameJson(JsonElement expected, JsonElement actual)
    {
        static JsonNode? Clean(JsonNode? node) => node switch
        {
            JsonObject o => new JsonObject(o.Where(p => p.Value is not null).OrderBy(p => p.Key, StringComparer.Ordinal).Select(p => KeyValuePair.Create(p.Key, Clean(p.Value?.DeepClone())))),
            JsonArray a => new JsonArray(a.Select(x => Clean(x?.DeepClone())).ToArray()),
            JsonValue v when v.TryGetValue<double>(out var d) => JsonValue.Create(Math.Round(d, 5)),
            // "Z" y "+00:00" son el mismo instante en RFC 3339: se comparan normalizados.
            JsonValue v when v.TryGetValue<string>(out var text) && text.Length >= 20 && text[10] == 'T'
                && DateTimeOffset.TryParse(text, System.Globalization.CultureInfo.InvariantCulture, System.Globalization.DateTimeStyles.None, out var instant)
                => JsonValue.Create(instant.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ss.fffZ", System.Globalization.CultureInfo.InvariantCulture)),
            _ => node?.DeepClone(),
        };

        Assert.Equal(Clean(JsonNode.Parse(expected.GetRawText()))!.ToJsonString(), Clean(JsonNode.Parse(actual.GetRawText()))!.ToJsonString());
    }

    [Fact]
    public void Protocol_version_matches()
    {
        Assert.Equal(SyncProtocol.Version, Vectors.GetProperty("protocol").GetString());
        Assert.Equal(SyncProtocol.Version, Read<PushRequest>("pushRequest").Protocol);
    }

    [Theory]
    [InlineData("pushRequest", typeof(PushRequest))]
    [InlineData("pushResponse", typeof(PushResponse))]
    [InlineData("pullRequest", typeof(PullRequest))]
    [InlineData("pullResponse", typeof(PullResponse))]
    [InlineData("conflict", typeof(SyncConflictDto))]
    [InlineData("resolveRequest", typeof(ResolveConflictRequest))]
    [InlineData("searchRequest", typeof(MemorySearchRequest))]
    [InlineData("searchResponse", typeof(MemorySearchResponse))]
    [InlineData("healthResponse", typeof(HealthResponse))]
    public void Examples_round_trip(string name, Type type)
    {
        var example = Vectors.GetProperty(name);
        var value = example.Deserialize(type, JarvisJson.Options);
        AssertSameJson(example, JsonSerializer.SerializeToElement(value, type, JarvisJson.Options));
    }

    [Theory]
    [InlineData("memory")]
    [InlineData("memoryWithDaysRetention")]
    public void Memory_examples_validate_and_map_back(string name)
    {
        var data = Vectors.GetProperty("entities").GetProperty(name).Deserialize<MemoryData>(JarvisJson.Options);
        var valid = SyncData.ValidateMemory(data, DateTimeOffset.UtcNow);
        Assert.True(valid.IsSuccess, valid.Error?.Message);
        var memory = new MemoryItem { SourceKind = valid.Value.Source.Kind!, SourceReference = valid.Value.Source.Reference, SourceLabel = valid.Value.Source.Label!, DeviceCreatedAt = valid.Value.CreatedAt, DeviceUpdatedAt = valid.Value.UpdatedAt };
        memory.Apply(valid.Value.Fields);
        AssertSameJson(Vectors.GetProperty("entities").GetProperty(name), SyncData.ToElement(SyncData.ToData(memory)));
    }

    [Fact]
    public void Other_entity_examples_validate()
    {
        var entities = Vectors.GetProperty("entities");
        Assert.True(SyncData.ValidateRelation(entities.GetProperty("relation").Deserialize<RelationData>(JarvisJson.Options)).IsSuccess);
        Assert.True(SyncData.ValidateEmbedding(entities.GetProperty("embedding").Deserialize<EmbeddingData>(JarvisJson.Options)).IsSuccess);
        Assert.True(SyncData.ValidateExperience(entities.GetProperty("experience").Deserialize<ExperienceData>(JarvisJson.Options), DateTimeOffset.UtcNow).IsSuccess);
        Assert.True(SyncData.ValidateSkill(entities.GetProperty("skill").Deserialize<SkillData>(JarvisJson.Options)).IsSuccess);
    }

    [Fact]
    public void Content_hashes_are_sha256_of_the_exact_utf8_content()
    {
        foreach (var vector in Vectors.GetProperty("contentHash").EnumerateArray())
        {
            var content = vector.GetProperty("content").GetString()!;
            Assert.Equal(vector.GetProperty("sha256").GetString(), Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(content))));
            Assert.Equal(vector.GetProperty("sha256").GetString(), SyncData.Hex(SyncData.ContentHash(content)));
        }
    }

    public static TheoryData<string> MergeCases()
    {
        var data = new TheoryData<string>();
        foreach (var c in Vectors.GetProperty("mergeCases").EnumerateArray())
        {
            data.Add(c.GetProperty("name").GetString()!);
        }

        return data;
    }

    private static MemoryFields Fields(JsonElement e) => new(
        "semantic",
        e.GetProperty("content").GetString()!,
        e.GetProperty("summary").GetString()!,
        e.GetProperty("tags").EnumerateArray().Select(t => t.GetString()!).ToList(),
        0.5f,
        e.GetProperty("confidence").GetSingle(),
        "verified",
        "permanent",
        null);

    [Theory]
    [MemberData(nameof(MergeCases))]
    public void Three_way_merge_follows_the_vectors(string name)
    {
        var c = Vectors.GetProperty("mergeCases").EnumerateArray().Single(x => x.GetProperty("name").GetString() == name);
        var outcome = ThreeWayMerge.Merge(Fields(c.GetProperty("base")), Fields(c.GetProperty("server")), Fields(c.GetProperty("device")));
        var expect = c.GetProperty("expect");
        if (expect.GetProperty("status").GetString() == "conflict")
        {
            Assert.True(outcome.IsConflict);
            Assert.Equal(expect.GetProperty("fields").EnumerateArray().Select(f => f.GetString()), outcome.ConflictingFields);
        }
        else
        {
            Assert.False(outcome.IsConflict);
            Assert.True(Fields(expect.GetProperty("result")).SameAs(outcome.Result!), name);
        }
    }

    [Theory]
    [InlineData("password: hunter22", true)]
    [InlineData("api_key=sk-abcdefghijklmnopqrstuvwxyz", true)]
    [InlineData("-----BEGIN OPENSSH PRIVATE KEY-----", true)]
    [InlineData("AKIAABCDEFGHIJKLMNOP", true)]
    [InlineData("La contraseña del wifi la sabe Ana", false)]
    [InlineData("El token de acceso caduca en una hora", false)]
    [InlineData("Mi servidor usa PostgreSQL 16", false)]
    public void Secret_guard_catches_credentials_without_flagging_prose(string text, bool secret) =>
        Assert.Equal(secret, SecretGuard.LooksLikeSecret(text));

    [Fact]
    public void Search_terms_drop_stop_words_and_are_safe_for_tsquery()
    {
        var terms = SyncData.SearchTerms("¿Qué base de datos usa el servidor? ' | & ! (x)");
        Assert.Equal(["base", "datos", "servidor"], terms);
        Assert.All(terms, t => Assert.Matches(@"^[\p{L}\p{N}]+$", t));
    }
}
