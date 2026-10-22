using Microsoft.AspNetCore.SignalR.Client;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Events;
using Jarvis.IntegrationTests.Infrastructure;
using Jarvis.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;

namespace Jarvis.IntegrationTests;

[Collection(JarvisCollection.Name)]
public abstract class IntegrationTest(JarvisTestEnvironment env, ITestOutputHelper output)
{
    protected JarvisTestEnvironment Env { get; } = env;

    protected ITestOutputHelper Output { get; } = output;

    protected async Task<OwnerSession> OwnerAsync()
    {
        Assert.SkipUnless(Env.Available, Env.UnavailableReason ?? "Docker unavailable");
        return await OwnerSession.CreateAsync(Env);
    }

    protected static StringContent Json(string json) => new(json, System.Text.Encoding.UTF8, "application/json");
}

/// <summary>HU-004 / HU-005 / HU-006 / HU-045.</summary>
public sealed class HostTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Health_live_and_ready()
    {
        Assert.SkipUnless(Env.Available, Env.UnavailableReason ?? string.Empty);
        var client = Env.CreateClient();
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/health/live")).StatusCode);
        await JarvisTestEnvironment.WaitForAsync(
            async () =>
            {
                var body = await client.GetStringAsync("/health/ready");
                return body.Contains("\"status\":\"Healthy\"", StringComparison.Ordinal) ? body : null;
            },
            TimeSpan.FromSeconds(20),
            "readiness to become Healthy");
    }

    [Fact]
    public async Task Errors_are_problem_details_with_correlation()
    {
        Assert.SkipUnless(Env.Available, Env.UnavailableReason ?? string.Empty);
        var client = Env.CreateClient();
        var correlation = Guid.CreateVersion7();
        using var request = new HttpRequestMessage(HttpMethod.Get, "/api/v1/devices");
        request.Headers.Add("X-Correlation-Id", correlation.ToString());
        var response = await client.SendAsync(request);
        await response.AssertProblemAsync(HttpStatusCode.Unauthorized, "UNAUTHENTICATED");
        Assert.Equal(correlation.ToString(), response.Headers.GetValues("X-Correlation-Id").Single());

        await (await client.GetAsync("/api/v1/does-not-exist")).AssertProblemAsync(HttpStatusCode.NotFound, "NOT_FOUND");
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync("/metrics")).StatusCode);
    }

    [Fact]
    public async Task Invalid_json_body_is_validation_problem()
    {
        var owner = await OwnerAsync();
        var response = await owner.Client.PostAsync("/api/v1/devices", Json("{not json"));
        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
    }

    [Fact]
    public async Task Body_over_64_kib_is_rejected()
    {
        var owner = await OwnerAsync();
        var response = await owner.Client.PostAsync("/api/v1/devices", Json("{\"name\":\"" + new string('a', 70 * 1024) + "\"}"));
        Assert.Equal(HttpStatusCode.RequestEntityTooLarge, response.StatusCode);
    }
}

/// <summary>HU-008 / HU-009 / HU-010 / HU-017.</summary>
public sealed class AuthTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Login_and_me()
    {
        var owner = await OwnerAsync();
        var me = await owner.Client.GetFromJsonAsync<MeResponse>("/api/v1/me", OwnerSession.Json);
        Assert.Equal("User", me!.PrincipalType);
        Assert.Equal(owner.UserId, me.Id);
        Assert.Equal(owner.UserId, me.OwnerId);
    }

    [Fact]
    public async Task Invalid_credentials_are_indistinguishable()
    {
        var owner = await OwnerAsync();
        var client = Env.CreateClient();
        await (await client.PostAsJsonAsync("/api/v1/auth/login", new LoginRequest(owner.Email, "wrong-password-123"))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_CREDENTIALS");
        await (await client.PostAsJsonAsync("/api/v1/auth/login", new LoginRequest("nobody@example.com", "wrong-password-123"))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_CREDENTIALS");
    }

    [Fact]
    public async Task Five_failures_lock_the_account()
    {
        var owner = await OwnerAsync();
        var client = Env.CreateClient();
        for (var i = 0; i < 5; i++)
        {
            await client.PostAsJsonAsync("/api/v1/auth/login", new LoginRequest(owner.Email, "wrong-password-123"));
        }

        await (await client.PostAsJsonAsync("/api/v1/auth/login", new LoginRequest(owner.Email, owner.Password))).AssertProblemAsync((HttpStatusCode)423, "ACCOUNT_LOCKED");
        var events = await owner.EventsAsync($"type={EventTypes.UserLockedOut}");
        Assert.Single(events.Items);
    }

    [Fact]
    public async Task Refresh_rotates_and_detects_reuse()
    {
        var owner = await OwnerAsync();
        var client = Env.CreateClient();
        var first = await client.PostAsJsonAsync("/api/v1/auth/refresh", new RefreshRequest(owner.Login.RefreshToken));
        Assert.Equal(HttpStatusCode.OK, first.StatusCode);
        var rotated = (await first.Content.ReadFromJsonAsync<LoginResponse>(OwnerSession.Json))!;

        await (await client.PostAsJsonAsync("/api/v1/auth/refresh", new RefreshRequest(owner.Login.RefreshToken))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_REFRESH_TOKEN");
        // La reutilización revocó toda la familia, incluido el token rotado.
        await (await client.PostAsJsonAsync("/api/v1/auth/refresh", new RefreshRequest(rotated.RefreshToken))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_REFRESH_TOKEN");
    }

    [Fact]
    public async Task Logout_revokes_refresh_token()
    {
        var owner = await OwnerAsync();
        Assert.Equal(HttpStatusCode.NoContent, (await owner.Client.PostAsJsonAsync("/api/v1/auth/logout", new LogoutRequest(owner.Login.RefreshToken))).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.Client.PostAsJsonAsync("/api/v1/auth/logout", new LogoutRequest(owner.Login.RefreshToken))).StatusCode);
        await (await Env.CreateClient().PostAsJsonAsync("/api/v1/auth/refresh", new RefreshRequest(owner.Login.RefreshToken))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_REFRESH_TOKEN");
    }

    [Fact]
    public async Task Device_token_lifecycle()
    {
        var owner = await OwnerAsync();
        var device = await owner.RegisterDeviceAsync("Windows");
        var deviceClient = await owner.DeviceClientAsync(device.Credential);
        var me = await deviceClient.GetFromJsonAsync<MeResponse>("/api/v1/me", OwnerSession.Json);
        Assert.Equal("Device", me!.PrincipalType);

        // Un token de device no accede a endpoints de user.
        await (await deviceClient.GetAsync("/api/v1/devices")).AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");
        await (await deviceClient.GetAsync("/api/v1/events")).AssertProblemAsync(HttpStatusCode.Forbidden, "FORBIDDEN");

        var client = Env.CreateClient();
        await (await client.PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(device.Device.Id, "jdv_wrong"))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_CREDENTIALS");
        await (await client.PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(Guid.NewGuid(), device.Credential.Secret))).AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_CREDENTIALS");

        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{device.Device.Id}/disable", null)).StatusCode);
        await (await client.PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(device.Device.Id, device.Credential.Secret))).AssertProblemAsync(HttpStatusCode.Forbidden, "DEVICE_DISABLED");

        // El token ya emitido deja de aceptarse (caché de estado de 2 s en tests).
        await JarvisTestEnvironment.WaitForAsync(
            async () => (await deviceClient.GetAsync("/api/v1/me")).StatusCode == HttpStatusCode.Unauthorized ? "rejected" : null,
            TimeSpan.FromSeconds(10),
            "device token to be rejected after disable");
    }
}

/// <summary>HU-012 … HU-016 / HU-025.</summary>
public sealed class DeviceTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Register_returns_secret_once_and_stores_only_hash()
    {
        var owner = await OwnerAsync();
        var response = await owner.Client.PostAsJsonAsync("/api/v1/devices", new CreateDeviceRequest("PC Escritorio", "Windows"));
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        Assert.Contains("no-store", response.Headers.CacheControl!.ToString(), StringComparison.Ordinal);
        var registered = (await response.Content.ReadFromJsonAsync<RegisterDeviceResponse>(OwnerSession.Json))!;
        Assert.StartsWith("jdv_", registered.Credential.Secret, StringComparison.Ordinal);
        Assert.Equal(Env.MqttTlsPort, registered.Credential.Mqtt.Port);
        Assert.True(registered.Device.BrokerSynced);

        var listed = await owner.Client.GetStringAsync("/api/v1/devices");
        Assert.DoesNotContain(registered.Credential.Secret, listed, StringComparison.Ordinal);

        await using var scope = Env.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
        var credential = await db.DeviceCredentials.IgnoreQueryFilters().SingleAsync(c => c.DeviceId == registered.Device.Id);
        Assert.Equal(32, credential.SecretHash!.Length);
        Assert.Equal(registered.Credential.Secret[^4..], credential.SecretHint);
    }

    [Fact]
    public async Task Validation_and_conflicts()
    {
        var owner = await OwnerAsync();
        await (await owner.Client.PostAsJsonAsync("/api/v1/devices", new CreateDeviceRequest("", "Windows"))).AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        await (await owner.Client.PostAsJsonAsync("/api/v1/devices", new CreateDeviceRequest("x", "Toaster"))).AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        await owner.RegisterDeviceAsync("ESP32", "Boton");
        await (await owner.Client.PostAsJsonAsync("/api/v1/devices", new CreateDeviceRequest("Boton", "ESP32"))).AssertProblemAsync(HttpStatusCode.Conflict, "DEVICE_NAME_TAKEN");

        var other = await OwnerAsync();
        await other.RegisterDeviceAsync("ESP32", "Boton");
    }

    [Fact]
    public async Task Rename_disable_enable_revoke()
    {
        var owner = await OwnerAsync();
        var device = await owner.RegisterDeviceAsync("Windows");
        var id = device.Device.Id;
        var renamed = await (await owner.Client.PatchAsJsonAsync($"/api/v1/devices/{id}", new RenameDeviceRequest("PC Oficina"))).Content.ReadFromJsonAsync<DeviceDto>(OwnerSession.Json);
        Assert.Equal("PC Oficina", renamed!.Name);

        var disabled = await (await owner.Client.PostAsync($"/api/v1/devices/{id}/disable", null)).Content.ReadFromJsonAsync<DeviceDto>(OwnerSession.Json);
        Assert.Equal("Disabled", disabled!.Status);
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{id}/disable", null)).StatusCode);
        var enabled = await (await owner.Client.PostAsync($"/api/v1/devices/{id}/enable", null)).Content.ReadFromJsonAsync<DeviceDto>(OwnerSession.Json);
        Assert.Equal("Active", enabled!.AdminState);

        var revoked = await (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{id}/revoke", new RevokeDeviceRequest("lost"))).Content.ReadFromJsonAsync<DeviceDto>(OwnerSession.Json);
        Assert.Equal("Revoked", revoked!.AdminState);
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{id}/revoke", new RevokeDeviceRequest(null))).StatusCode);
        await (await owner.Client.PostAsync($"/api/v1/devices/{id}/enable", null)).AssertProblemAsync(HttpStatusCode.Conflict, "DEVICE_REVOKED");

        var events = await owner.EventsAsync($"deviceId={id}");
        var types = events.Items.Select(e => e.Type).ToList();
        Assert.Contains(EventTypes.DeviceRegistered, types);
        Assert.Contains(EventTypes.DeviceUpdated, types);
        Assert.Contains(EventTypes.DeviceDisabled, types);
        Assert.Contains(EventTypes.DeviceEnabled, types);
        Assert.Contains(EventTypes.DeviceRevoked, types);
        Assert.Single(types, t => t == EventTypes.DeviceDisabled);
    }

    [Fact]
    public async Task Disable_disconnects_and_rotation_invalidates_old_secret()
    {
        var owner = await OwnerAsync();
        var device = await owner.RegisterDeviceAsync("Windows");
        await using var windows = owner.StartWindows(device.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        await owner.WaitForDeviceStatusAsync(device.Device.Id, "Online");

        var rotate = await owner.Client.PostAsync($"/api/v1/devices/{device.Device.Id}/credentials/rotate", null);
        Assert.Equal(HttpStatusCode.OK, rotate.StatusCode);
        var newCredential = (await rotate.Content.ReadFromJsonAsync<IssuedCredentialDto>(OwnerSession.Json))!;
        Assert.NotEqual(device.Credential.Secret, newCredential.Secret);

        // La sesión MQTT con el secreto viejo se cierra y no puede reconectar.
        await JarvisTestEnvironment.WaitUntilAsync(() => !windows.Connection.IsConnected, TimeSpan.FromSeconds(10), "old session to be disconnected");
        await Task.Delay(3000);
        Assert.False(windows.Connection.IsConnected);
        await (await Env.CreateClient().PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(device.Device.Id, device.Credential.Secret)))
            .AssertProblemAsync(HttpStatusCode.Unauthorized, "INVALID_CREDENTIALS");

        await using var reconnected = owner.StartWindows(newCredential, output: Output);
        await reconnected.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{device.Device.Id}/disable", null)).StatusCode);
        await JarvisTestEnvironment.WaitUntilAsync(() => !reconnected.Connection.IsConnected, TimeSpan.FromSeconds(5), "disabled device to be disconnected");
    }

    [Fact]
    public async Task Grants_validation()
    {
        var owner = await OwnerAsync();
        var esp = await owner.RegisterDeviceAsync("ESP32");
        var pc = await owner.RegisterDeviceAsync("Windows");
        var grant = await owner.GrantAsync(esp.Device.Id, pc.Device.Id);
        await (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{esp.Device.Id}/grants", new CreateGrantRequest(pc.Device.Id, "OPEN_CHROME"))).AssertProblemAsync(HttpStatusCode.Conflict, "GRANT_ALREADY_EXISTS");
        await (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{esp.Device.Id}/grants", new CreateGrantRequest(pc.Device.Id, "*"))).AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        await (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{esp.Device.Id}/grants", new CreateGrantRequest(esp.Device.Id, "OPEN_CHROME"))).AssertProblemAsync((HttpStatusCode)422, "SAME_DEVICE");

        var other = await OwnerAsync();
        var foreign = await other.RegisterDeviceAsync("Windows");
        await (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{esp.Device.Id}/grants", new CreateGrantRequest(foreign.Device.Id, "OPEN_CHROME"))).AssertProblemAsync(HttpStatusCode.NotFound, "NOT_FOUND");

        Assert.Equal(HttpStatusCode.NoContent, (await owner.Client.DeleteAsync($"/api/v1/devices/{esp.Device.Id}/grants/{grant.Id}")).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.Client.DeleteAsync($"/api/v1/devices/{esp.Device.Id}/grants/{grant.Id}")).StatusCode);
    }
}

/// <summary>HU-020: aislamiento de topics entre devices (ACL por device en Dynamic Security).</summary>
public sealed class BrokerAclTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Device_cannot_touch_other_device_topics()
    {
        var owner = await OwnerAsync();
        var a = await owner.RegisterDeviceAsync("ESP32");
        var b = await owner.RegisterDeviceAsync("Windows");
        await using var esp = owner.StartEsp32(a.Credential, b.Device.Id, Output);
        await esp.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var bUp = Contracts.Mqtt.MqttTopics.Up(b.Device.Id, "tasks");
        Assert.False(await esp.Connection.TrySubscribeRawAsync(Contracts.Mqtt.MqttTopics.DeviceSubscription(b.Device.Id)));
        Assert.False(await esp.Connection.TrySubscribeRawAsync("jarvis/v1/devices/+/up/#"));
        Assert.False(await esp.Connection.TryPublishRawAsync(bUp, "{}"u8.ToArray()));
        Assert.False(await esp.Connection.TryPublishRawAsync(Contracts.Mqtt.MqttTopics.Down(b.Device.Id, "tasks"), "{}"u8.ToArray()));
        Assert.False(await esp.Connection.TryPublishRawAsync("$CONTROL/dynamic-security/v1", "{}"u8.ToArray()));
        Assert.True(await esp.Connection.TryPublishRawAsync(Contracts.Mqtt.MqttTopics.Up(a.Device.Id, "events"), "{}"u8.ToArray()));
    }

    [Fact]
    public async Task Envelope_device_mismatch_is_a_security_violation()
    {
        var owner = await OwnerAsync();
        var a = await owner.RegisterDeviceAsync("ESP32");
        var b = await owner.RegisterDeviceAsync("Windows");
        await using var esp = owner.StartEsp32(a.Credential, b.Device.Id, Output);
        await esp.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var forged = Contracts.Mqtt.MqttEnvelope.Create(
            Contracts.Mqtt.MessageTypes.PresenceHeartbeat, b.Device.Id, new Contracts.Mqtt.PresenceHeartbeatData("s-x", false, 0), DateTimeOffset.UtcNow);
        await esp.Connection.PublishEnvelopeAsync("presence", forged);
        var violation = await JarvisTestEnvironment.WaitForAsync(
            async () => (await owner.EventsAsync($"type={EventTypes.SecurityViolation}&deviceId={a.Device.Id}")).Items.FirstOrDefault(),
            TimeSpan.FromSeconds(10),
            "SECURITY_VIOLATION event");
        Assert.Equal("topic_device_mismatch", violation.Data.GetProperty("rule").GetString());
    }
}

/// <summary>HU-026 / HU-032 / HU-033 / HU-039.</summary>
public sealed class TaskApiTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Create_validations_and_idempotency()
    {
        var owner = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");

        // El target aún no declaró supportedTaskTypes (no envió hello).
        await (await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id)).AssertProblemAsync((HttpStatusCode)422, "TASK_TYPE_NOT_SUPPORTED");

        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");

        await (await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, type: "open_chrome")).AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        await (await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, payload: new { big = new string('x', 17 * 1024) })).AssertProblemAsync(HttpStatusCode.RequestEntityTooLarge, "PAYLOAD_TOO_LARGE");
        await (await OwnerSession.PostTaskAsync(owner.Client, Guid.NewGuid())).AssertProblemAsync(HttpStatusCode.NotFound, "NOT_FOUND");

        var first = await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, idempotencyKey: "rest-key-1");
        Assert.Equal(HttpStatusCode.Created, first.StatusCode);
        var task = (await first.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!;
        var replay = await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, idempotencyKey: "rest-key-1");
        Assert.Equal(HttpStatusCode.OK, replay.StatusCode);
        Assert.Equal("true", replay.Headers.GetValues("Idempotent-Replayed").Single());
        Assert.Equal(task.Id, (await replay.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!.Id);
        await (await OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, payload: new { url = "https://other.example" }, idempotencyKey: "rest-key-1"))
            .AssertProblemAsync((HttpStatusCode)422, "IDEMPOTENCY_KEY_REUSED");

        var completed = await owner.WaitForTaskStatusAsync(task.Id, "Completed");
        Assert.Equal(1, windows.ExecutionCount(task.Id));
        Assert.Equal("SUCCESS", completed.Result!.Value.GetProperty("outcome").GetString());
        await (await owner.Client.PostAsync($"/api/v1/tasks/{task.Id}/cancel", null)).AssertProblemAsync(HttpStatusCode.Conflict, "INVALID_STATE_TRANSITION");
    }

    [Fact]
    public async Task Concurrent_requests_with_same_key_create_one_task()
    {
        var owner = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");

        var responses = await Task.WhenAll(Enumerable.Range(0, 6).Select(_ => OwnerSession.PostTaskAsync(owner.Client, pc.Device.Id, idempotencyKey: "same-key")));
        var ids = new HashSet<Guid>();
        foreach (var response in responses)
        {
            Assert.True(response.StatusCode is HttpStatusCode.Created or HttpStatusCode.OK, response.StatusCode.ToString());
            ids.Add((await response.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!.Id);
        }

        Assert.Single(ids);
    }

    [Fact]
    public async Task Device_requires_grant_and_idempotency_key()
    {
        var owner = await OwnerAsync();
        var esp = await owner.RegisterDeviceAsync("ESP32");
        var pc = await owner.RegisterDeviceAsync("Windows");
        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        var deviceClient = await owner.DeviceClientAsync(esp.Credential);

        await (await OwnerSession.PostTaskAsync(deviceClient, pc.Device.Id)).AssertProblemAsync(HttpStatusCode.BadRequest, "IDEMPOTENCY_KEY_REQUIRED");
        await (await OwnerSession.PostTaskAsync(deviceClient, pc.Device.Id, idempotencyKey: "k1")).AssertProblemAsync(HttpStatusCode.Forbidden, "TASK_NOT_GRANTED");
        Assert.NotEmpty((await owner.EventsAsync($"type={EventTypes.TaskRejected}")).Items);

        await owner.GrantAsync(esp.Device.Id, pc.Device.Id);
        var created = await OwnerSession.PostTaskAsync(deviceClient, pc.Device.Id, idempotencyKey: "k2");
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        var task = (await created.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!;
        Assert.Equal(esp.Device.Id, task.SourceDeviceId);
        Assert.Equal("Device", task.RequestedBy.Type);

        // El device solo ve tareas donde es origen o target.
        var mine = await deviceClient.GetFromJsonAsync<Page<TaskDto>>("/api/v1/tasks", OwnerSession.Json);
        Assert.All(mine!.Items, t => Assert.True(t.SourceDeviceId == esp.Device.Id || t.TargetDeviceId == esp.Device.Id));
        await (await deviceClient.PostAsync($"/api/v1/tasks/{Guid.NewGuid()}/cancel", null)).AssertProblemAsync(HttpStatusCode.NotFound, "NOT_FOUND");
    }

    [Fact]
    public async Task Cancel_pending_task_is_immediate_and_listing_filters()
    {
        var owner = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        await using (var windows = owner.StartWindows(pc.Credential, output: Output))
        {
            await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
            await windows.StopAsync();
        }

        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Offline");
        var task = await owner.CreateTaskAsync(pc.Device.Id);
        Assert.Equal("Pending", task.Status);
        var cancel = await owner.Client.PostAsync($"/api/v1/tasks/{task.Id}/cancel", null);
        Assert.Equal(HttpStatusCode.OK, cancel.StatusCode);
        Assert.Equal("Cancelled", (await cancel.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!.Status);

        var cancelled = await owner.Client.GetFromJsonAsync<Page<TaskDto>>("/api/v1/tasks?status=Cancelled&limit=1", OwnerSession.Json);
        Assert.Equal(task.Id, Assert.Single(cancelled!.Items).Id);
        Assert.Null(cancelled.Items[0].Payload);
        await (await owner.Client.GetAsync("/api/v1/tasks?status=Bogus")).AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
    }

    [Fact]
    public async Task Pagination_with_cursor()
    {
        var owner = await OwnerAsync();
        for (var i = 0; i < 5; i++)
        {
            await owner.RegisterDeviceAsync("ESP32", $"esp-{i}");
        }

        var seen = new List<Guid>();
        string? cursor = null;
        do
        {
            var page = await owner.Client.GetFromJsonAsync<Page<DeviceDto>>($"/api/v1/devices?limit=2{(cursor is null ? string.Empty : "&cursor=" + cursor)}", OwnerSession.Json);
            seen.AddRange(page!.Items.Select(d => d.Id));
            cursor = page.NextCursor;
        }
        while (cursor is not null);

        Assert.Equal(5, seen.Distinct().Count());
    }
}

/// <summary>HU-043: todos los recursos de otro owner responden 404 (nunca revelan existencia).</summary>
public sealed class CrossOwnerAuthorizationTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Other_owner_gets_not_found_everywhere()
    {
        var owner = await OwnerAsync();
        var attacker = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        var esp = await owner.RegisterDeviceAsync("ESP32");
        var grant = await owner.GrantAsync(esp.Device.Id, pc.Device.Id);
        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        var task = await owner.CreateTaskAsync(pc.Device.Id);

        var id = pc.Device.Id;
        var requests = new (HttpMethod Method, string Url, object? Body)[]
        {
            (HttpMethod.Get, $"/api/v1/devices/{id}", null),
            (HttpMethod.Patch, $"/api/v1/devices/{id}", new { name = "pwned" }),
            (HttpMethod.Post, $"/api/v1/devices/{id}/disable", null),
            (HttpMethod.Post, $"/api/v1/devices/{id}/enable", null),
            (HttpMethod.Post, $"/api/v1/devices/{id}/revoke", new { }),
            (HttpMethod.Post, $"/api/v1/devices/{id}/credentials/rotate", null),
            (HttpMethod.Get, $"/api/v1/devices/{esp.Device.Id}/grants", null),
            (HttpMethod.Post, $"/api/v1/devices/{esp.Device.Id}/grants", new { targetDeviceId = id, taskType = "OPEN_CHROME" }),
            (HttpMethod.Delete, $"/api/v1/devices/{esp.Device.Id}/grants/{grant.Id}", null),
            (HttpMethod.Get, $"/api/v1/tasks/{task.Id}", null),
            (HttpMethod.Post, $"/api/v1/tasks/{task.Id}/cancel", null),
            (HttpMethod.Post, "/api/v1/tasks", new { targetDeviceId = id, type = "OPEN_CHROME", payload = new { } }),
        };

        foreach (var (method, url, body) in requests)
        {
            using var request = new HttpRequestMessage(method, url) { Content = body is null ? null : JsonContent.Create(body) };
            var response = await attacker.Client.SendAsync(request);
            Assert.True(response.StatusCode == HttpStatusCode.NotFound, $"{method} {url} returned {response.StatusCode}");
        }

        var devices = await attacker.Client.GetFromJsonAsync<Page<DeviceDto>>("/api/v1/devices", OwnerSession.Json);
        Assert.Empty(devices!.Items);
        var tasks = await attacker.Client.GetFromJsonAsync<Page<TaskDto>>("/api/v1/tasks", OwnerSession.Json);
        Assert.Empty(tasks!.Items);
        var events = await attacker.EventsAsync($"taskId={task.Id}");
        Assert.Empty(events.Items);

        // Nada cambió para el owner legítimo.
        var device = await owner.GetDeviceAsync(id);
        Assert.Equal("Active", device.AdminState);
        Assert.NotEqual("pwned", device.Name);
    }
}

/// <summary>HU-046: ningún secreto ni contenido de payload/result en logs ni en events.data.</summary>
public sealed class RedactionTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Secrets_and_payloads_never_reach_logs_or_events()
    {
        var owner = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        var deviceClient = await owner.DeviceClientAsync(pc.Credential);
        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        var marker = "payload-marker-" + Guid.NewGuid().ToString("N");
        var task = await owner.CreateTaskAsync(pc.Device.Id, payload: new { url = "https://example.com/" + marker });
        await owner.WaitForTaskStatusAsync(task.Id, "Completed");
        var rotated = await owner.Client.PostAsync($"/api/v1/devices/{pc.Device.Id}/credentials/rotate", null);
        var newSecret = (await rotated.Content.ReadFromJsonAsync<IssuedCredentialDto>(OwnerSession.Json))!.Secret;

        var logs = Env.Logs.AllText();
        string[] sensitive =
        [
            pc.Credential.Secret, newSecret, owner.Password, owner.Login.AccessToken, owner.Login.RefreshToken,
            deviceClient.DefaultRequestHeaders.Authorization!.Parameter!, marker, "Chrome opened (simulated)",
        ];
        foreach (var value in sensitive)
        {
            Assert.DoesNotContain(value, logs, StringComparison.Ordinal);
        }

        await using var scope = Env.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
        var data = await db.Events.IgnoreQueryFilters().Where(e => e.OwnerId == owner.UserId).Select(e => e.Data).ToListAsync();
        var allData = string.Join('\n', data);
        foreach (var value in sensitive)
        {
            Assert.DoesNotContain(value, allData, StringComparison.Ordinal);
        }
    }
}

/// <summary>HU-041 / HU-042: notificaciones SignalR por owner y catch-up por REST.</summary>
public sealed class RealtimeTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Owner_receives_task_notifications_and_other_owner_does_not()
    {
        var owner = await OwnerAsync();
        var other = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        await using var windows = owner.StartWindows(pc.Credential, output: Output);
        await windows.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var received = new System.Collections.Concurrent.ConcurrentQueue<RealtimeNotification>();
        var leaked = new System.Collections.Concurrent.ConcurrentQueue<RealtimeNotification>();
        await using var hub = Connect(owner.Login.AccessToken, received);
        await using var otherHub = Connect(other.Login.AccessToken, leaked);
        await hub.StartAsync();
        await otherHub.StartAsync();

        var task = await owner.CreateTaskAsync(pc.Device.Id);
        await JarvisTestEnvironment.WaitUntilAsync(
            () => received.Any(n => n.TaskId == task.Id && n.Name == "task.completed"),
            TimeSpan.FromSeconds(20),
            "task.completed notification");
        var names = received.Where(n => n.TaskId == task.Id).Select(n => n.Name).ToList();
        Assert.Equal(["task.created", "task.dispatched", "task.delivered", "task.started", "task.completed"], names);
        Assert.Empty(leaked);

        // Catch-up: los eventos después del primer seq coinciden con lo notificado.
        var firstSeq = received.First(n => n.TaskId == task.Id).Seq!.Value;
        var catchUp = await owner.EventsAsync($"after={firstSeq - 1}&taskId={task.Id}");
        Assert.Equal(received.Where(n => n.TaskId == task.Id).Select(n => n.Seq!.Value), catchUp.Items.Select(e => e.Seq));
    }

    [Fact]
    public async Task Device_tokens_cannot_connect_to_hub()
    {
        var owner = await OwnerAsync();
        var pc = await owner.RegisterDeviceAsync("Windows");
        var deviceClient = await owner.DeviceClientAsync(pc.Credential);
        await using var hub = Connect(deviceClient.DefaultRequestHeaders.Authorization!.Parameter!, new());
        await Assert.ThrowsAnyAsync<Exception>(() => hub.StartAsync());
    }

    private Microsoft.AspNetCore.SignalR.Client.HubConnection Connect(string token, System.Collections.Concurrent.ConcurrentQueue<RealtimeNotification> sink)
    {
        var connection = new Microsoft.AspNetCore.SignalR.Client.HubConnectionBuilder()
            .WithUrl(new Uri(Env.ServerBaseAddress, "/hubs/events"), options =>
            {
                options.HttpMessageHandlerFactory = _ => Env.CreateHandler();
                options.Transports = Microsoft.AspNetCore.Http.Connections.HttpTransportType.LongPolling;
                options.AccessTokenProvider = () => Task.FromResult<string?>(token);
            })
            .AddJsonProtocol(o => o.PayloadSerializerOptions.Converters.Add(new System.Text.Json.Serialization.JsonStringEnumConverter()))
            .Build();
        connection.On<RealtimeNotification>("event", sink.Enqueue);
        return connection;
    }
}
