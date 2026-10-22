using System.Net;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Jobs;
using Jarvis.Contracts.Rest;
using Jarvis.Contracts.Sync;
using Jarvis.Infrastructure.Persistence;
using Jarvis.IntegrationTests.Infrastructure;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Logging.Abstractions;

namespace Jarvis.IntegrationTests;

/// <summary>Device emparejado para sincronizar (cliente con token de device y scopes concedidos).</summary>
public sealed record SyncDevice(Guid Id, HttpClient Client, IssuedCredentialDto Credential);

public static class SyncHelpers
{
    public static readonly string[] AllScopes = ["memories:read", "memories:write", "experiences:read", "experiences:write", "skills:read", "skills:write"];

    public static async Task<SyncDevice> PairAsync(this OwnerSession owner, string name, string[]? scopes = null)
    {
        var registered = await owner.RegisterDeviceAsync("Windows", name);
        var put = await owner.Client.PutAsJsonAsync($"/api/v1/sync/devices/{registered.Device.Id}", new { scopes = scopes ?? AllScopes });
        Assert.Equal(HttpStatusCode.OK, put.StatusCode);
        var client = await owner.DeviceClientAsync(registered.Credential);
        return new SyncDevice(registered.Device.Id, client, registered.Credential);
    }

    public static JsonObject Memory(string content, string summary = "", string[]? tags = null, float confidence = 0.9f, string verification = "verified", string kind = "semantic", object? retention = null) =>
        new()
        {
            ["kind"] = kind,
            ["content"] = content,
            ["summary"] = summary,
            ["tags"] = new JsonArray((tags ?? []).Select(t => (JsonNode)t).ToArray()),
            ["relevance"] = 0.5,
            ["confidence"] = confidence,
            ["verification"] = verification,
            ["retention"] = retention is null ? new JsonObject { ["kind"] = "permanent" } : JsonSerializer.SerializeToNode(retention),
            ["source"] = new JsonObject { ["kind"] = "user", ["label"] = "usuario" },
            ["createdAt"] = "2026-10-22T15:00:00Z",
            ["updatedAt"] = "2026-10-22T15:00:00Z",
        };

    public static JsonObject Change(string entity, Guid entityId, string op, int? baseVersion, JsonNode? data, Guid? changeId = null) =>
        new()
        {
            ["changeId"] = (changeId ?? Guid.NewGuid()).ToString(),
            ["entity"] = entity,
            ["entityId"] = entityId.ToString(),
            ["op"] = op,
            ["baseVersion"] = baseVersion,
            ["data"] = data,
        };

    public static async Task<PushResponse> PushAsync(this SyncDevice device, params JsonObject[] changes)
    {
        var response = await device.PushRawAsync(changes);
        var body = await response.Content.ReadAsStringAsync();
        Assert.True(response.StatusCode == HttpStatusCode.OK, body);
        return JsonSerializer.Deserialize<PushResponse>(body, OwnerSession.Json)!;
    }

    public static Task<HttpResponseMessage> PushRawAsync(this SyncDevice device, params JsonObject[] changes) =>
        device.Client.PostAsJsonAsync("/api/v1/sync/push", new JsonObject { ["protocol"] = "jarvis.sync/1", ["changes"] = new JsonArray(changes) });

    public static async Task<PullResponse> PullAsync(this SyncDevice device, long since = 0, string[]? models = null)
    {
        var response = await device.Client.PostAsJsonAsync("/api/v1/sync/pull", new { protocol = "jarvis.sync/1", since, limit = 500, embeddingModels = models ?? [] });
        var body = await response.Content.ReadAsStringAsync();
        Assert.True(response.StatusCode == HttpStatusCode.OK, body);
        return JsonSerializer.Deserialize<PullResponse>(body, OwnerSession.Json)!;
    }

    public static string Sha(string content) => Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(content)));
}

/// <summary>Sincronización de memoria (ADR-013, ADR-015, sync-protocol.md) contra PostgreSQL real.</summary>
public sealed class SyncTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Memory_created_on_pc_a_is_pulled_and_found_on_pc_b()
    {
        var owner = await OwnerAsync();
        var pcA = await owner.PairAsync("pc-a");
        var pcB = await owner.PairAsync("pc-b");
        var vps = Guid.NewGuid();
        var postgres = Guid.NewGuid();
        var relation = Guid.NewGuid();

        var push = await pcA.PushAsync(
            SyncHelpers.Change("memory", vps, "upsert", null, SyncHelpers.Memory("Máquina de Contabo", kind: "entity")),
            SyncHelpers.Change("memory", postgres, "upsert", null, SyncHelpers.Memory("El servidor VPS usa PostgreSQL 16", "VPS con PostgreSQL", ["infra"])),
            SyncHelpers.Change("relation", relation, "upsert", null, new JsonObject
            {
                ["sourceId"] = postgres.ToString(), ["targetId"] = vps.ToString(), ["kind"] = "BELONGS_TO",
                ["confidence"] = 0.9, ["provenance"] = "user", ["verified"] = true,
            }),
            SyncHelpers.Change("embedding", postgres, "upsert", null, new JsonObject
            {
                ["model"] = "test-embed", ["dims"] = 4, ["contentHash"] = SyncHelpers.Sha("El servidor VPS usa PostgreSQL 16"),
                ["vector"] = new JsonArray(0.9, 0.1, 0.0, 0.1),
            }));
        Assert.All(push.Results, r => Assert.Equal("applied", r.Status));
        Assert.True(push.Cursor > 0);

        var pulled = await pcB.PullAsync(models: ["test-embed"]);
        Assert.Contains(pulled.Changes, c => c.Entity == "memory" && c.EntityId == postgres && c.Version == 1 && c.OriginDeviceId == pcA.Id);
        Assert.Contains(pulled.Changes, c => c.Entity == "relation" && c.EntityId == relation);
        var embedding = Assert.Single(pulled.Changes, c => c.Entity == "embedding");
        Assert.Equal(4, embedding.Data!.Value.GetProperty("vector").GetArrayLength());
        Assert.Equal([.. pulled.Changes.Select(c => c.Seq).Order()], pulled.Changes.Select(c => c.Seq).ToArray());

        // Recuperación desde PC B con un vector del mismo modelo: aparece el recuerdo y, por el grafo, el VPS.
        var search = await pcB.Client.PostAsJsonAsync("/api/v1/memories/search", new
        {
            query = "¿qué base de datos usa el servidor?",
            embedding = new { model = "test-embed", vector = new[] { 1.0f, 0.0f, 0.0f, 0.0f } },
        });
        var result = (await search.Content.ReadFromJsonAsync<MemorySearchResponse>(OwnerSession.Json))!;
        Assert.Equal(Env.VectorExtension ? "vector:pgvector" : "vector:application", result.Method);
        Assert.Equal(postgres, result.Memories[0].Id);
        Assert.Contains(result.Memories, m => m.Id == vps && m.Via == "graph");
        Assert.Contains(result.Relations, r => r.Id == relation);

        // Sin vector: búsqueda de texto.
        var text = await (await pcB.Client.PostAsJsonAsync("/api/v1/memories/search", new { query = "PostgreSQL" })).Content.ReadFromJsonAsync<MemorySearchResponse>(OwnerSession.Json);
        Assert.Equal("text", text!.Method);
        Assert.Equal(postgres, text.Memories[0].Id);
    }

    [Fact]
    public async Task Application_vector_search_is_used_without_pgvector()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-app");
        var id = Guid.NewGuid();
        await pc.PushAsync(
            SyncHelpers.Change("memory", id, "upsert", null, SyncHelpers.Memory("Mi color favorito es el verde")),
            SyncHelpers.Change("embedding", id, "upsert", null, new JsonObject
            {
                ["model"] = "app-embed", ["dims"] = 3, ["contentHash"] = SyncHelpers.Sha("Mi color favorito es el verde"), ["vector"] = new JsonArray(0.0, 1.0, 0.0),
            }));
        MemorySql.VectorOverride = false;
        try
        {
            var response = await pc.Client.PostAsJsonAsync("/api/v1/memories/search", new { query = "color", embedding = new { model = "app-embed", vector = new[] { 0.0f, 0.9f, 0.1f } } });
            var result = (await response.Content.ReadFromJsonAsync<MemorySearchResponse>(OwnerSession.Json))!;
            Assert.Equal("vector:application", result.Method);
            Assert.Equal(id, Assert.Single(result.Memories).Id);
        }
        finally
        {
            MemorySql.VectorOverride = null;
        }
    }

    [Fact]
    public async Task Pushing_the_same_change_twice_is_idempotent()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-retry");
        var change = SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Recordatorio repetido"));

        var first = await pc.PushAsync(change);
        var second = await pc.PushAsync(change.DeepClone().AsObject());
        Assert.False(first.Results[0].Replayed);
        Assert.True(second.Results[0].Replayed);
        Assert.Equal(first.Results[0].Version, second.Results[0].Version);
        Assert.Equal(first.Results[0].Seq, second.Results[0].Seq);

        await using var scope = Env.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
        Assert.Equal(1, await db.MemoryVersions.IgnoreQueryFilters().CountAsync(v => v.MemoryId == first.Results[0].EntityId));
    }

    [Fact]
    public async Task Concurrent_edits_merge_disjoint_fields_and_conflict_on_the_same_field()
    {
        var owner = await OwnerAsync();
        var pcA = await owner.PairAsync("pc-a");
        var pcB = await owner.PairAsync("pc-b");
        var id = Guid.NewGuid();
        await pcA.PushAsync(SyncHelpers.Change("memory", id, "upsert", null, SyncHelpers.Memory("El VPS usa PostgreSQL 16", tags: ["infra"], confidence: 0.5f)));

        // A cambia el contenido; B (todavía en la versión 1) cambia la confianza y añade una etiqueta: se fusionan.
        var a = await pcA.PushAsync(SyncHelpers.Change("memory", id, "upsert", 1, SyncHelpers.Memory("El VPS usa PostgreSQL 17", tags: ["infra"], confidence: 0.5f)));
        Assert.Equal("applied", a.Results[0].Status);
        var b = await pcB.PushAsync(SyncHelpers.Change("memory", id, "upsert", 1, SyncHelpers.Memory("El VPS usa PostgreSQL 16", tags: ["infra", "db"], confidence: 0.95f)));
        var merged = b.Results[0];
        Assert.Equal("merged", merged.Status);
        Assert.Equal(3, merged.Version);
        Assert.Equal("El VPS usa PostgreSQL 17", merged.Data!.Value.GetProperty("content").GetString());
        Assert.Equal(0.95, merged.Data.Value.GetProperty("confidence").GetDouble(), 3);
        Assert.Equal(["db", "infra"], merged.Data.Value.GetProperty("tags").EnumerateArray().Select(t => t.GetString()).ToArray());

        // Ahora ambos editan el contenido a partir de la versión 3: el segundo recibe un conflicto y nada se pisa.
        await pcA.PushAsync(SyncHelpers.Change("memory", id, "upsert", 3, SyncHelpers.Memory("Producción usa PostgreSQL 17.2", tags: ["db", "infra"], confidence: 0.95f)));
        var conflict = (await pcB.PushAsync(SyncHelpers.Change("memory", id, "upsert", 3, SyncHelpers.Memory("Producción usa PostgreSQL 16.4", tags: ["db", "infra"], confidence: 0.95f)))).Results[0];
        Assert.Equal("conflict", conflict.Status);
        Assert.Equal("update_update", conflict.ConflictKind);
        Assert.Equal(["content"], conflict.Fields);

        var detail = await owner.Client.GetFromJsonAsync<MemoryDetailResponse>($"/api/v1/memories/{id}", OwnerSession.Json);
        Assert.Equal("Producción usa PostgreSQL 17.2", detail!.Memory.Content);
        Assert.True(detail.Memory.HasOpenConflict);

        // Se resuelve con un contenido nuevo: versión nueva que llega a todos por pull.
        var resolve = await owner.Client.PostAsJsonAsync($"/api/v1/sync/conflicts/{conflict.ConflictId}/resolve", new
        {
            resolution = "custom",
            data = SyncHelpers.Memory("Producción migró de PostgreSQL 16.4 a 17.2", tags: ["db", "infra"]),
        });
        var resolved = (await resolve.Content.ReadFromJsonAsync<ResolveConflictResponse>(OwnerSession.Json))!;
        Assert.Equal(5, resolved.Version);
        var again = await owner.Client.PostAsJsonAsync($"/api/v1/sync/conflicts/{conflict.ConflictId}/resolve", new { resolution = "keep_server" });
        await again.AssertProblemAsync(HttpStatusCode.Conflict, "CONFLICT_ALREADY_RESOLVED");

        var pulled = await pcB.PullAsync(since: conflict.Version ?? 0);
        var latest = pulled.Changes.Last(c => c.EntityId == id);
        Assert.Equal(5, latest.Version);
        Assert.Equal("Producción migró de PostgreSQL 16.4 a 17.2", latest.Data!.Value.GetProperty("content").GetString());
        var versions = await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/memories/{id}/versions");
        Assert.Equal(5, versions.GetProperty("items").GetArrayLength());
    }

    [Fact]
    public async Task Deletes_are_tombstones_that_purge_content_and_edits_after_delete_conflict()
    {
        var owner = await OwnerAsync();
        var pcA = await owner.PairAsync("pc-a");
        var pcB = await owner.PairAsync("pc-b");
        var id = Guid.NewGuid();
        var other = Guid.NewGuid();
        await pcA.PushAsync(
            SyncHelpers.Change("memory", id, "upsert", null, SyncHelpers.Memory("Dato que se va a borrar")),
            SyncHelpers.Change("memory", other, "upsert", null, SyncHelpers.Memory("Otro dato")),
            SyncHelpers.Change("relation", Guid.NewGuid(), "upsert", null, new JsonObject
            {
                ["sourceId"] = id.ToString(), ["targetId"] = other.ToString(), ["kind"] = "RELATED_TO", ["provenance"] = "user", ["verified"] = true,
            }));
        var cursor = (await pcB.PullAsync()).NextSince;

        var deleted = await pcA.PushAsync(SyncHelpers.Change("memory", id, "delete", 1, null));
        Assert.Equal("applied", deleted.Results[0].Status);
        Assert.Equal(2, deleted.Results[0].Version);

        var pulled = await pcB.PullAsync(cursor);
        var tombstone = Assert.Single(pulled.Changes, c => c.Entity == "memory");
        Assert.Equal("delete", tombstone.Op);
        Assert.Null(tombstone.Data);
        Assert.Contains(pulled.Changes, c => c.Entity == "relation" && c.Op == "delete");

        await using (var scope = Env.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
            var row = await db.Memories.IgnoreQueryFilters().AsNoTracking().FirstAsync(m => m.Id == id);
            Assert.Null(row.Content);
            Assert.Empty(row.Tags);
            Assert.False(await db.MemoryVersions.IgnoreQueryFilters().AnyAsync(v => v.MemoryId == id));
        }

        // PC B editó sin saber del borrado: conflicto, se guarda su propuesta y el tombstone se mantiene.
        var edit = (await pcB.PushAsync(SyncHelpers.Change("memory", id, "upsert", 1, SyncHelpers.Memory("Edición tardía")))).Results[0];
        Assert.Equal("conflict", edit.Status);
        Assert.Equal("update_delete", edit.ConflictKind);
        var conflicts = await owner.Client.GetFromJsonAsync<JsonElement>("/api/v1/sync/conflicts");
        Assert.Contains(conflicts.GetProperty("items").EnumerateArray(), c => c.GetProperty("deviceData").GetProperty("content").GetString() == "Edición tardía");

        // Borrar algo que otro editó también es conflicto.
        await pcA.PushAsync(SyncHelpers.Change("memory", other, "upsert", 1, SyncHelpers.Memory("Otro dato, editado")));
        var stale = (await pcB.PushAsync(SyncHelpers.Change("memory", other, "delete", 1, null))).Results[0];
        Assert.Equal("delete_update", stale.ConflictKind);
    }

    [Fact]
    public async Task Invalid_changes_are_rejected_one_by_one()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-validation");
        var id = Guid.NewGuid();
        var response = await pc.PushAsync(
            SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("password: hunter2-super-secreto")),
            SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Recuerdo de sesión", retention: new { kind = "session" })),
            SyncHelpers.Change("memory", id, "upsert", null, SyncHelpers.Memory("Válido")),
            SyncHelpers.Change("embedding", id, "upsert", null, new JsonObject { ["model"] = "m", ["dims"] = 2, ["contentHash"] = SyncHelpers.Sha("otro contenido"), ["vector"] = new JsonArray(1.0, 0.0) }),
            SyncHelpers.Change("relation", Guid.NewGuid(), "upsert", null, new JsonObject { ["sourceId"] = id.ToString(), ["targetId"] = Guid.NewGuid().ToString(), ["kind"] = "RELATED_TO" }),
            SyncHelpers.Change("relation", Guid.NewGuid(), "upsert", null, new JsonObject { ["sourceId"] = id.ToString(), ["targetId"] = id.ToString(), ["kind"] = "RELATED_TO" }),
            SyncHelpers.Change("memory", id, "upsert", 7, SyncHelpers.Memory("Desde el futuro")));
        Assert.Equal(["CONTENT_REJECTED", "VALIDATION_FAILED", null, "STALE_EMBEDDING", "REFERENCE_NOT_FOUND", "VALIDATION_FAILED", "VALIDATION_FAILED"], response.Results.Select(r => r.Code).ToArray());
        Assert.Equal("applied", response.Results[2].Status);

        var wrongProtocol = await pc.Client.PostAsJsonAsync("/api/v1/sync/push", new { protocol = "jarvis.sync/9", changes = Array.Empty<object>() });
        await wrongProtocol.AssertProblemAsync(HttpStatusCode.BadRequest, "UNSUPPORTED_PROTOCOL");
    }

    [Fact]
    public async Task Devices_need_scopes_users_cannot_push_and_revoked_devices_stop_syncing()
    {
        var owner = await OwnerAsync();
        var readOnly = await owner.PairAsync("pc-readonly", ["memories:read"]);
        var rejected = await readOnly.PushAsync(SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("No debería entrar")));
        Assert.Equal("SYNC_SCOPE_MISSING", rejected.Results[0].Code);
        var pull = await readOnly.Client.PostAsJsonAsync("/api/v1/sync/pull", new { protocol = "jarvis.sync/1", entities = new[] { "skill" } });
        await pull.AssertProblemAsync(HttpStatusCode.Forbidden, "SYNC_SCOPE_MISSING");

        var userPush = await owner.Client.PostAsJsonAsync("/api/v1/sync/push", new { protocol = "jarvis.sync/1", changes = Array.Empty<object>() });
        await userPush.AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");

        var pc = await owner.PairAsync("pc-revoked");
        await pc.PushAsync(SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Antes de revocar")));
        var revoke = await owner.Client.PostAsJsonAsync($"/api/v1/devices/{pc.Id}/revoke", new { reason = "PC robada" });
        Assert.Equal(HttpStatusCode.OK, revoke.StatusCode);
        await JarvisTestEnvironment.WaitForAsync(
            async () => (await pc.PushRawAsync(SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Después")))).StatusCode == HttpStatusCode.Unauthorized ? "ok" : null,
            TimeSpan.FromSeconds(15),
            "revoked device to be rejected");
        var token = await Env.CreateClient().PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(pc.Id, pc.Credential.Secret));
        Assert.Equal(HttpStatusCode.Unauthorized, token.StatusCode);
    }

    [Fact]
    public async Task Access_check_accepts_active_users_and_devices_and_rejects_revoked_ones()
    {
        // Lo usa nginx (auth_request) para /ollama, /stt y /tts: misma credencial que la API.
        var owner = await OwnerAsync();
        Assert.Equal(HttpStatusCode.NoContent, (await owner.Client.GetAsync("/api/v1/access/check")).StatusCode);
        Assert.Equal(HttpStatusCode.Unauthorized, (await Env.CreateClient().GetAsync("/api/v1/access/check")).StatusCode);
        var anonymous = Env.CreateClient();
        anonymous.DefaultRequestHeaders.Authorization = new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer", "no-es-un-jwt");
        Assert.Equal(HttpStatusCode.Unauthorized, (await anonymous.GetAsync("/api/v1/access/check")).StatusCode);

        var pc = await owner.PairAsync("pc-access");
        Assert.Equal(HttpStatusCode.NoContent, (await pc.Client.GetAsync("/api/v1/access/check")).StatusCode);
        var revoke = await owner.Client.PostAsJsonAsync($"/api/v1/devices/{pc.Id}/revoke", new { reason = "PC robada" });
        Assert.Equal(HttpStatusCode.OK, revoke.StatusCode);
        await JarvisTestEnvironment.WaitForAsync(
            async () => (await pc.Client.GetAsync("/api/v1/access/check")).StatusCode == HttpStatusCode.Unauthorized ? "ok" : null,
            TimeSpan.FromSeconds(40),
            "revoked device to lose access to the owner services");
    }

    [Fact]
    public async Task Push_body_limit_is_one_mebibyte_and_other_endpoints_keep_64_kib()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-size");
        var changes = new List<JsonObject>();
        for (var i = 0; i < 40; i++)
        {
            var id = Guid.NewGuid();
            var content = $"Recuerdo {i} " + new string('x', 1500);
            changes.Add(SyncHelpers.Change("memory", id, "upsert", null, SyncHelpers.Memory(content)));
            changes.Add(SyncHelpers.Change("embedding", id, "upsert", null, new JsonObject
            {
                ["model"] = "big", ["dims"] = 1024, ["contentHash"] = SyncHelpers.Sha(content),
                ["vector"] = new JsonArray(Enumerable.Range(0, 1024).Select(j => (JsonNode)(j / 1024.0)).ToArray()),
            }));
        }

        var body = new JsonObject { ["protocol"] = "jarvis.sync/1", ["changes"] = new JsonArray([.. changes]) }.ToJsonString();
        Assert.True(Encoding.UTF8.GetByteCount(body) is > 64 * 1024 and < 1024 * 1024);
        var ok = await pc.Client.PostAsync("/api/v1/sync/push", new StringContent(body, Encoding.UTF8, "application/json"));
        Assert.Equal(HttpStatusCode.OK, ok.StatusCode);
        var results = (await ok.Content.ReadFromJsonAsync<PushResponse>(OwnerSession.Json))!.Results;
        Assert.All(results, r => Assert.Equal("applied", r.Status));

        var tooBig = await pc.Client.PostAsync("/api/v1/sync/push", new StringContent(new string(' ', 1024 * 1024 + 10), Encoding.UTF8, "application/json"));
        await tooBig.AssertProblemAsync(HttpStatusCode.RequestEntityTooLarge, "PAYLOAD_TOO_LARGE");
        var search = await pc.Client.PostAsync("/api/v1/memories/search", new StringContent(new string(' ', 70 * 1024), Encoding.UTF8, "application/json"));
        await search.AssertProblemAsync(HttpStatusCode.RequestEntityTooLarge, "PAYLOAD_TOO_LARGE");
    }

    [Fact]
    public async Task Skills_from_devices_start_as_drafts_and_only_the_owner_verifies_them()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-skills");
        var skillId = Guid.NewGuid();
        var skill = new JsonObject
        {
            ["name"] = "Abrir el bloc de notas", ["description"] = "Abre Notepad", ["procedure"] = "1. open_app {\"appId\":\"notepad\"}",
            ["status"] = "verified", ["compatibility"] = new JsonObject { ["platforms"] = new JsonArray("windows"), ["requiredTools"] = new JsonArray("open_app") },
            ["permissions"] = new JsonObject { ["maxRisk"] = "R1" }, ["source"] = "device",
        };
        var pushed = (await pc.PushAsync(SyncHelpers.Change("skill", skillId, "upsert", null, skill.DeepClone()))).Results[0];
        Assert.Equal("merged", pushed.Status);
        Assert.Equal("draft", pushed.Data!.Value.GetProperty("status").GetString());

        var verify = await owner.Client.PatchAsJsonAsync($"/api/v1/skills/{skillId}", new { status = "verified" });
        Assert.Equal(HttpStatusCode.OK, verify.StatusCode);
        var deviceVerify = await pc.Client.PatchAsJsonAsync($"/api/v1/skills/{skillId}", new { status = "verified" });
        await deviceVerify.AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");

        // Cambiar el procedimiento crea una versión nueva y la vuelve a dejar en borrador.
        skill["procedure"] = "1. open_app {\"appId\":\"notepad\"}\n2. get_active_window {}";
        skill["status"] = "verified";
        var updated = (await pc.PushAsync(SyncHelpers.Change("skill", skillId, "upsert", 2, skill.DeepClone()))).Results[0];
        Assert.Equal("merged", updated.Status);
        Assert.Equal("draft", updated.Data!.Value.GetProperty("status").GetString());
        var versions = await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/skills/{skillId}/versions");
        Assert.Equal(2, versions.GetProperty("items").GetArrayLength());
    }

    [Fact]
    public async Task Export_contains_live_records_with_a_verifiable_hash()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-export");
        await pc.PushAsync(
            SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Exportable 1")),
            SyncHelpers.Change("memory", Guid.NewGuid(), "upsert", null, SyncHelpers.Memory("Exportable 2")));
        var export = (await owner.Client.GetFromJsonAsync<MemoryExport>("/api/v1/memories/export", OwnerSession.Json))!;
        Assert.Equal("jarvis.memory-export/1", export.Format);
        Assert.Equal(2, export.Memories.Count);
        Assert.Equal(Jarvis.Application.Sync.MemoryQueryHandler.ExportHash(export.Memories, export.Relations, export.Experiences, export.Skills), export.Sha256);
        var deviceExport = await pc.Client.GetAsync("/api/v1/memories/export");
        await deviceExport.AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");
    }

    [Fact]
    public async Task Health_reports_protocol_and_vector_search()
    {
        var health = await Env.CreateClient().GetFromJsonAsync<HealthResponse>("/api/v1/health", OwnerSession.Json);
        Assert.Equal("jarvis.sync/1", health!.Features.Sync);
        Assert.Equal(Env.VectorExtension ? "pgvector" : "application", health.Features.VectorSearch);
    }
}

/// <summary>Jobs y worker (ADR-016) con el planificador real sobre PostgreSQL.</summary>
public sealed class JobTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    private async Task<(JobScheduler Scheduler, AsyncServiceScope Scope)> SchedulerAsync(string workerId, TimeProvider? clock = null)
    {
        var scope = Env.Services.CreateAsyncScope();
        var sp = scope.ServiceProvider;
        var db = sp.GetRequiredService<IJarvisDbContext>();
        var sql = sp.GetRequiredService<IMemorySql>();
        var time = clock ?? TimeProvider.System;
        IJobHandler[] handlers =
        [
            new ReportSummaryJob(db, time),
            new MemoryConsolidateJob(db, sql, time),
            new SyncCompactJob(db, time),
            new MemoryPurgeExpiredJob(db, sql, time),
        ];
        var scheduler = new JobScheduler(db, sql, sp.GetRequiredService<IRequestContext>(), handlers, sp, new JobSchedulerOptions { WorkerId = workerId, LeaseGraceSeconds = 1 }, time, NullLogger<JobScheduler>.Instance);
        await Task.CompletedTask;
        return (scheduler, scope);
    }

    private async Task<JobRunDto[]> RunAllAsync(Guid jobId, OwnerSession owner)
    {
        var (scheduler, scope) = await SchedulerAsync("tests");
        await using (scope)
        {
            await scheduler.ScheduleDueAsync(CancellationToken.None);
            foreach (var run in await scheduler.ClaimAsync(CancellationToken.None))
            {
                await scheduler.ExecuteAsync(run, CancellationToken.None);
            }
        }

        var runs = await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/jobs/{jobId}/runs");
        return JsonSerializer.Deserialize<JobRunDto[]>(runs.GetProperty("items").GetRawText(), OwnerSession.Json)!;
    }

    [Fact]
    public async Task Sensitive_jobs_wait_for_approval_and_retention_creates_tombstones()
    {
        var owner = await OwnerAsync();
        var pc = await owner.PairAsync("pc-retention");
        var expired = Guid.NewGuid();
        var kept = Guid.NewGuid();
        var old = SyncHelpers.Memory("Nota temporal de hace tiempo", retention: new { kind = "days", days = 1 });
        old["createdAt"] = DateTimeOffset.UtcNow.AddDays(-3).ToString("O");
        await pc.PushAsync(
            SyncHelpers.Change("memory", expired, "upsert", null, old),
            SyncHelpers.Change("memory", kept, "upsert", null, SyncHelpers.Memory("Nota permanente")));
        var cursor = (await pc.PullAsync()).NextSince;

        var create = await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "memory.purge_expired", name = "Retención" });
        var job = (await create.Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;
        Assert.Equal("pending", job.ApprovalState);
        var early = await owner.Client.PostAsync($"/api/v1/jobs/{job.Id}/run", null);
        await early.AssertProblemAsync(HttpStatusCode.Conflict, "JOB_NOT_RUNNABLE");
        Assert.Empty(await RunAllAsync(job.Id, owner));

        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/jobs/{job.Id}/approve", null)).StatusCode);
        var run = Assert.Single(await RunAllAsync(job.Id, owner));
        Assert.Equal("succeeded", run.Status);
        Assert.Equal(1, run.Result!.Value.GetProperty("tombstoned").GetInt32());

        var pulled = await pc.PullAsync(cursor);
        var tombstone = Assert.Single(pulled.Changes);
        Assert.Equal(expired, tombstone.EntityId);
        Assert.Equal("delete", tombstone.Op);
    }

    [Fact]
    public async Task Consolidation_suggests_duplicates_and_skill_proposals_without_deleting()
    {
        var owner = await OwnerAsync();
        var pcA = await owner.PairAsync("pc-a");
        var pcB = await owner.PairAsync("pc-b");
        var first = Guid.NewGuid();
        var second = Guid.NewGuid();
        await pcA.PushAsync(SyncHelpers.Change("memory", first, "upsert", null, SyncHelpers.Memory("Mi editor es VS Code")));
        await pcB.PushAsync(SyncHelpers.Change("memory", second, "upsert", null, SyncHelpers.Memory("Mi editor es VS Code")));
        for (var i = 0; i < 2; i++)
        {
            await pcA.PushAsync(SyncHelpers.Change("experience", Guid.NewGuid(), "upsert", null, new JsonObject
            {
                ["goal"] = "Abrir el bloc de notas", ["steps"] = new JsonArray(new JsonObject { ["tool"] = "open_app", ["arguments"] = new JsonObject { ["appId"] = "notepad" }, ["ok"] = true, ["verification"] = "PASSED" }),
                ["result"] = "Abierto", ["procedure"] = "1. open_app {\"appId\":\"notepad\"}", ["verified"] = true, ["completed"] = true,
                ["occurrences"] = 2, ["signature"] = "sig-notepad",
            }));
        }

        var job = (await (await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "memory.consolidate", parameters = new { minSupport = 3 } }))
            .Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;
        Assert.Equal("not_required", job.ApprovalState);
        var run = Assert.Single(await RunAllAsync(job.Id, owner));
        Assert.Equal("succeeded", run.Status);
        Assert.Equal(1, run.Result!.Value.GetProperty("duplicateRelations").GetInt32());
        Assert.Equal(1, run.Result.Value.GetProperty("skillProposals").GetInt32());

        var detail = await owner.Client.GetFromJsonAsync<MemoryDetailResponse>($"/api/v1/memories/{second}", OwnerSession.Json);
        var duplicate = Assert.Single(detail!.Relations);
        Assert.Equal("DUPLICATE_OF", duplicate.Kind);
        Assert.False(duplicate.Verified);
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.GetAsync($"/api/v1/memories/{first}")).StatusCode);

        var proposals = await owner.Client.GetFromJsonAsync<JsonElement>("/api/v1/skills/proposals");
        var proposal = proposals.GetProperty("items")[0];
        Assert.Equal(4, proposal.GetProperty("supportCount").GetInt32());
        var accepted = await owner.Client.PostAsync($"/api/v1/skills/proposals/{proposal.GetProperty("id").GetGuid()}/accept", null);
        Assert.Equal(HttpStatusCode.OK, accepted.StatusCode);
        var skills = await pcA.Client.GetFromJsonAsync<JsonElement>("/api/v1/skills");
        Assert.Equal("draft", skills.GetProperty("items")[0].GetProperty("status").GetString());

        var similar = await pcB.Client.GetFromJsonAsync<JsonElement>("/api/v1/experiences/similar?goal=abrir%20bloc%20de%20notas");
        Assert.Equal(2, similar.GetProperty("items").GetArrayLength());
    }

    [Fact]
    public async Task A_run_lost_by_a_crashed_worker_is_reclaimed_after_its_lease()
    {
        var owner = await OwnerAsync();
        var job = (await (await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "report.summary", timeoutSeconds = 1, maxAttempts = 3 }))
            .Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;

        // Worker 1 reclama la ejecución y "muere" sin ejecutarla.
        var (crashed, scope1) = await SchedulerAsync("worker-1");
        await using (scope1)
        {
            await crashed.ScheduleDueAsync(CancellationToken.None);
            Assert.Single(await crashed.ClaimAsync(CancellationToken.None));
        }

        // Mientras la lease está vigente nadie más la toma; al vencer, worker 2 la reintenta y termina.
        var (other, scope2) = await SchedulerAsync("worker-2");
        await using (scope2)
        {
            Assert.Empty(await other.ClaimAsync(CancellationToken.None));
        }

        await Task.Delay(TimeSpan.FromSeconds(2.5));
        var (rescuer, scope3) = await SchedulerAsync("worker-2");
        await using (scope3)
        {
            var claimed = Assert.Single(await rescuer.ClaimAsync(CancellationToken.None));
            await rescuer.ExecuteAsync(claimed, CancellationToken.None);
        }

        var run = Assert.Single((await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/jobs/{job.Id}/runs")).GetProperty("items").EnumerateArray());
        Assert.Equal("succeeded", run.GetProperty("status").GetString());
        Assert.Equal(2, run.GetProperty("attempt").GetInt32());
    }

    [Fact]
    public async Task Unknown_job_types_and_devices_are_refused()
    {
        var owner = await OwnerAsync();
        var unknown = await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "shell.run", parameters = new { command = "rm -rf /" } });
        await unknown.AssertProblemAsync(HttpStatusCode.UnprocessableEntity, "JOB_TYPE_UNKNOWN");
        var pc = await owner.PairAsync("pc-jobs");
        var device = await pc.Client.GetAsync("/api/v1/jobs");
        await device.AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");
    }
}

/// <summary>El proceso jarvis-worker real (host con su bucle) contra la misma base.</summary>
public sealed class WorkerProcessTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    private Microsoft.Extensions.Hosting.IHost StartWorker(string backups) =>
        Jarvis.Worker.WorkerHost.CreateBuilder([], config => config.AddInMemoryCollection(new Dictionary<string, string?>
        {
            ["Logging:Console:FormatterName"] = "simple",
            ["ConnectionStrings:Postgres"] = Env.PostgresConnectionString,
            ["Jarvis:Worker:PollSeconds"] = "1",
            ["Jarvis:Worker:BackupDirectory"] = backups,
            ["Jarvis:Worker:HeartbeatPath"] = Path.Combine(backups, "..", $"hb-{Guid.NewGuid():N}"),
        })).Build();

    [Fact]
    public async Task Jobs_created_while_the_worker_is_down_run_when_it_comes_back()
    {
        var owner = await OwnerAsync();
        var backups = Path.Combine(Env.StateDirectory, $"backups-{Guid.NewGuid():N}");
        Directory.CreateDirectory(backups);
        var dump = Path.Combine(backups, "jarvis-20261022T030000Z.dump.age");
        await File.WriteAllTextAsync(dump, "age-encryption.org/v1\n" + new string('x', 200));
        await File.WriteAllTextAsync(Path.Combine(backups, "SHA256SUMS"), $"{Convert.ToHexStringLower(SHA256.HashData(await File.ReadAllBytesAsync(dump)))}  {Path.GetFileName(dump)}\n");
        await File.WriteAllTextAsync(Path.Combine(backups, ".last-success"), string.Empty);

        // Worker apagado: los jobs quedan persistidos.
        var report = (await (await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "report.summary" })).Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;
        var backup = (await (await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "backup.verify", parameters = new { maxAgeHours = 2 } })).Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;

        using var worker = StartWorker(backups);
        await worker.StartAsync();
        try
        {
            foreach (var job in new[] { report, backup })
            {
                var run = await JarvisTestEnvironment.WaitForAsync(
                    async () =>
                    {
                        var runs = await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/jobs/{job.Id}/runs");
                        var items = runs.GetProperty("items");
                        return items.GetArrayLength() > 0 && items[0].GetProperty("status").GetString() is "succeeded" or "failed" ? items[0].GetRawText() : null;
                    },
                    TimeSpan.FromSeconds(20),
                    $"{job.Type} to finish");
                Assert.True(JsonDocument.Parse(run).RootElement.GetProperty("status").GetString() == "succeeded", run);
            }
        }
        finally
        {
            await worker.StopAsync();
        }

        // Un checksum que no coincide hace fallar la verificación (con reintentos y error registrado).
        await File.WriteAllTextAsync(dump, "age-encryption.org/v1\n" + new string('y', 200));
        var tampered = (await (await owner.Client.PostAsJsonAsync("/api/v1/jobs", new { type = "backup.verify", maxAttempts = 1 })).Content.ReadFromJsonAsync<JobDto>(OwnerSession.Json))!;
        using var again = StartWorker(backups);
        await again.StartAsync();
        try
        {
            var failed = await JarvisTestEnvironment.WaitForAsync(
                async () =>
                {
                    var items = (await owner.Client.GetFromJsonAsync<JsonElement>($"/api/v1/jobs/{tampered.Id}/runs")).GetProperty("items");
                    return items.GetArrayLength() > 0 && items[0].GetProperty("status").GetString() == "failed" ? items[0].GetRawText() : null;
                },
                TimeSpan.FromSeconds(20),
                "tampered backup to fail");
            Assert.Contains("checksum mismatch", JsonDocument.Parse(failed).RootElement.GetProperty("error").GetString());
        }
        finally
        {
            await again.StopAsync();
        }
    }
}
