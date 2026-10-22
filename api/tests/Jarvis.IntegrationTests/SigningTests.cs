using System.Net;
using System.Net.Http.Json;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Events;
using Jarvis.IntegrationTests.Infrastructure;
using Jarvis.Simulators;

namespace Jarvis.IntegrationTests;

/// <summary>
/// ADR-012 y HU-057 desde el borde del Cloud.
///
/// Lo que se prueba aquí es <b>transporte y reglas</b>: que la firma llegue íntegra al device y que
/// las decisiones se registren y se retransmitan. La verificación criptográfica vive en el agente
/// (windows-agent, HU-WIN-061/062) y se prueba allí; el simulador de estas pruebas no verifica nada.
/// </summary>
public sealed class SigningTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    private static string SignatureValue(byte fill = 7) => Convert.ToBase64String(Enumerable.Repeat(fill, 64).ToArray());

    private static object Signature(string keyId = "owner-laptop-1", byte fill = 7) =>
        new { alg = "Ed25519", keyId, value = SignatureValue(fill) };

    private static Task<HttpResponseMessage> PostSignedAsync(
        HttpClient client,
        Guid target,
        Guid? taskId,
        DateTimeOffset? expiresAt,
        object? signature,
        string type = "OPEN_CHROME") =>
        client.PostAsJsonAsync("/api/v1/tasks", new
        {
            taskId,
            targetDeviceId = target,
            type,
            payload = new { url = "https://example.com" },
            expiresAt,
            signature,
        });

    [Fact]
    public async Task A_signed_task_reaches_the_device_with_its_signature_intact()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        await using var simulator = owner.StartWindows(windows.Credential, output: Output);
        await simulator.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var taskId = Guid.CreateVersion7();
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5);
        var response = await PostSignedAsync(owner.Client, windows.Device.Id, taskId, expiresAt, Signature());

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var created = (await response.Content.ReadFromJsonAsync<TaskDto>(OwnerSession.Json))!;
        // El id y la expiración son los que el cliente firmó: si el Cloud los cambiara, la firma
        // dejaría de validar en el agente y la tarea moriría con SIGNATURE_INVALID.
        Assert.Equal(taskId, created.Id);
        Assert.Equal(expiresAt.ToUnixTimeMilliseconds(), created.ExpiresAt.ToUnixTimeMilliseconds());
        Assert.Equal("owner-laptop-1", created.Signature!.KeyId);

        await owner.WaitForTaskStatusAsync(taskId, "Completed");
        var relayed = simulator.DispatchSignatures[taskId];
        Assert.NotNull(relayed);
        Assert.Equal("Ed25519", relayed.Alg);
        Assert.Equal("owner-laptop-1", relayed.KeyId);
        Assert.Equal(SignatureValue(), relayed.Value);
    }

    [Fact]
    public async Task An_unsigned_task_still_arrives_without_a_signature()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        await using var simulator = owner.StartWindows(windows.Credential, output: Output);
        await simulator.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var task = await owner.CreateTaskAsync(windows.Device.Id);

        await owner.WaitForTaskStatusAsync(task.Id, "Completed");
        Assert.Null(task.Signature);
        Assert.Null(simulator.DispatchSignatures[task.Id]);
    }

    [Fact]
    public async Task A_signed_task_must_carry_the_id_and_expiry_that_were_signed()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5);

        var noId = await PostSignedAsync(owner.Client, windows.Device.Id, null, expiresAt, Signature());
        await noId.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");

        var noExpiry = await PostSignedAsync(owner.Client, windows.Device.Id, Guid.CreateVersion7(), null, Signature());
        await noExpiry.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");

        // Y sin firma el cliente no elige el id: sería regalar la capacidad de ocupar ids ajenos.
        var idWithoutSignature = await PostSignedAsync(owner.Client, windows.Device.Id, Guid.CreateVersion7(), expiresAt, null);
        await idWithoutSignature.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
    }

    [Fact]
    public async Task A_malformed_signature_is_refused_by_shape()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5);

        foreach (var bad in new object[]
        {
            new { alg = "RS256", keyId = "owner-laptop-1", value = SignatureValue() },
            new { alg = "Ed25519", keyId = "clave con espacios", value = SignatureValue() },
            new { alg = "Ed25519", keyId = "owner-laptop-1", value = "no-es-base64!!" },
            // 32 bytes es una clave pública, no una firma.
            new { alg = "Ed25519", keyId = "owner-laptop-1", value = Convert.ToBase64String(new byte[32]) },
        })
        {
            var response = await PostSignedAsync(owner.Client, windows.Device.Id, Guid.CreateVersion7(), expiresAt, bad);
            await response.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        }
    }

    [Fact]
    public async Task A_task_id_can_only_be_used_once()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        // El device tiene que estar conectado: hasta el presence.hello no declara qué tipos soporta.
        await using var simulator = owner.StartWindows(windows.Credential, output: Output);
        await simulator.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        var taskId = Guid.CreateVersion7();
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5);

        var first = await PostSignedAsync(owner.Client, windows.Device.Id, taskId, expiresAt, Signature());
        Assert.Equal(HttpStatusCode.Created, first.StatusCode);

        // Otra firma sobre el mismo id: no se pisa la tarea original.
        var second = await PostSignedAsync(owner.Client, windows.Device.Id, taskId, expiresAt, Signature(fill: 9));
        await second.AssertProblemAsync(HttpStatusCode.Conflict, "TASK_ID_IN_USE");
    }

    [Fact]
    public async Task An_expiry_outside_the_window_is_refused()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");

        var tooSoon = await PostSignedAsync(owner.Client, windows.Device.Id, Guid.CreateVersion7(), DateTimeOffset.UtcNow.AddSeconds(2), Signature());
        await tooSoon.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");

        var tooLate = await PostSignedAsync(owner.Client, windows.Device.Id, Guid.CreateVersion7(), DateTimeOffset.UtcNow.AddDays(8), Signature());
        await tooLate.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
    }

    // ---- HU-057: aprobación remota -----------------------------------------------

    private static object Decision(string decision = "GRANTED", string? nonce = null, DateTimeOffset? decidedAt = null, object? signature = null) =>
        new
        {
            decision,
            nonce = nonce ?? $"n-{Guid.NewGuid():N}",
            decidedAt = decidedAt ?? DateTimeOffset.UtcNow,
            signature = signature ?? Signature(),
        };

    private async Task<(OwnerSession Owner, WindowsSimulator Simulator, Guid TaskId, Guid DeviceId)> WaitingTaskAsync()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        var simulator = owner.StartWindows(
            windows.Credential,
            new WindowsSimulatorOptions
            {
                ExecutionDelay = TimeSpan.FromMilliseconds(200),
                OutboxResendInterval = TimeSpan.FromSeconds(3),
                RequireApprovalTypes = new HashSet<string> { "OPEN_CHROME" },
            },
            Output);
        await simulator.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));

        var task = await owner.CreateTaskAsync(windows.Device.Id);
        await owner.WaitForTaskStatusAsync(task.Id, "WaitingForApproval");
        return (owner, simulator, task.Id, windows.Device.Id);
    }

    [Fact]
    public async Task A_signed_decision_is_relayed_to_the_device_verbatim()
    {
        var (owner, simulator, taskId, _) = await WaitingTaskAsync();
        await using var _ = simulator;

        var decidedAt = DateTimeOffset.UtcNow;
        var nonce = $"n-{Guid.NewGuid():N}";
        var response = await owner.Client.PostAsJsonAsync(
            $"/api/v1/tasks/{taskId}/approval",
            Decision(nonce: nonce, decidedAt: decidedAt));

        // 202 significa "transmitida", no "aprobada": quien decide es el agente.
        Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
        var approval = (await response.Content.ReadFromJsonAsync<TaskApprovalDto>(OwnerSession.Json))!;
        Assert.Equal("GRANTED", approval.Decision);

        var delivered = await JarvisTestEnvironment.WaitForAsync(
            () => Task.FromResult(simulator.ApprovalDecisions.FirstOrDefault(d => d.TaskId == taskId)),
            TimeSpan.FromSeconds(15),
            "the device to receive the decision");

        // Estos son exactamente los campos que el agente vuelve a meter en la cadena firmada.
        Assert.Equal("GRANTED", delivered.Decision);
        Assert.Equal(nonce, delivered.Nonce);
        Assert.Equal(decidedAt.ToUnixTimeMilliseconds(), delivered.DecidedAt.ToUnixTimeMilliseconds());
        Assert.Equal("owner-laptop-1", delivered.Signature.KeyId);
        Assert.Equal(SignatureValue(), delivered.Signature.Value);

        await owner.WaitForTaskStatusAsync(taskId, "Completed");
        // Dos hechos distintos: el owner firmó y el Cloud transmitió (SENT), y el device la aplicó
        // (GRANTED). Si la firma no cuadrara, habría SENT y nunca GRANTED.
        var sent = await owner.EventsAsync($"type={EventTypes.ApprovalDecisionSent}&taskId={taskId}");
        Assert.Single(sent.Items);
        Assert.Equal("owner-laptop-1", sent.Items[0].Data.GetProperty("signatureKeyId").GetString());
        var applied = await owner.EventsAsync($"type={EventTypes.ApprovalGranted}&taskId={taskId}");
        Assert.Single(applied.Items);
    }

    [Fact]
    public async Task A_denial_travels_the_same_way()
    {
        var (owner, simulator, taskId, _) = await WaitingTaskAsync();
        await using var _ = simulator;

        var response = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{taskId}/approval", Decision("DENIED"));

        Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
        await owner.WaitForTaskStatusAsync(taskId, "Cancelled");
        var sent = await owner.EventsAsync($"type={EventTypes.ApprovalDecisionSent}&taskId={taskId}");
        Assert.Single(sent.Items);
        Assert.Equal("DENIED", sent.Items[0].Data.GetProperty("decision").GetString());
    }

    [Fact]
    public async Task A_decision_without_a_signature_never_leaves_the_cloud()
    {
        var (owner, simulator, taskId, _) = await WaitingTaskAsync();
        await using var _ = simulator;

        var response = await owner.Client.PostAsJsonAsync(
            $"/api/v1/tasks/{taskId}/approval",
            new { decision = "GRANTED", nonce = "n-sin-firma", decidedAt = DateTimeOffset.UtcNow });

        // Sin firma el agente la ignoraría: mejor decirlo aquí que dejar la tarea colgada.
        await response.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
        Assert.DoesNotContain(simulator.ApprovalDecisions, d => d.TaskId == taskId);
    }

    [Fact]
    public async Task A_task_is_decided_only_once()
    {
        var (owner, simulator, taskId, _) = await WaitingTaskAsync();
        await using var _ = simulator;

        var first = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{taskId}/approval", Decision());
        Assert.Equal(HttpStatusCode.Accepted, first.StatusCode);

        var second = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{taskId}/approval", Decision("DENIED"));
        await second.AssertProblemAsync(HttpStatusCode.Conflict, "APPROVAL_ALREADY_DECIDED");
    }

    [Fact]
    public async Task A_nonce_is_never_reused()
    {
        var (owner, simulator, taskId, deviceId) = await WaitingTaskAsync();
        await using var _ = simulator;
        var nonce = $"n-{Guid.NewGuid():N}";

        var first = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{taskId}/approval", Decision(nonce: nonce));
        Assert.Equal(HttpStatusCode.Accepted, first.StatusCode);

        // Otra tarea del mismo owner, el mismo nonce: es la señal de una decisión capturada y reenviada.
        var other = await owner.CreateTaskAsync(deviceId);
        await owner.WaitForTaskStatusAsync(other.Id, "WaitingForApproval");
        var replay = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{other.Id}/approval", Decision(nonce: nonce));
        await replay.AssertProblemAsync(HttpStatusCode.Conflict, "NONCE_REUSED");
    }

    [Fact]
    public async Task A_stale_decision_is_refused()
    {
        var (owner, simulator, taskId, _) = await WaitingTaskAsync();
        await using var _ = simulator;

        var response = await owner.Client.PostAsJsonAsync(
            $"/api/v1/tasks/{taskId}/approval",
            Decision(decidedAt: DateTimeOffset.UtcNow.AddMinutes(-30)));

        await response.AssertProblemAsync(HttpStatusCode.BadRequest, "VALIDATION_FAILED");
    }

    [Fact]
    public async Task A_task_that_is_not_waiting_cannot_be_decided()
    {
        var owner = await OwnerAsync();
        var windows = await owner.RegisterDeviceAsync("Windows");
        await using var simulator = owner.StartWindows(windows.Credential, output: Output);
        await simulator.Connection.WaitReadyAsync(TimeSpan.FromSeconds(20));
        var task = await owner.CreateTaskAsync(windows.Device.Id);
        await owner.WaitForTaskStatusAsync(task.Id, "Completed");

        var response = await owner.Client.PostAsJsonAsync($"/api/v1/tasks/{task.Id}/approval", Decision());

        await response.AssertProblemAsync(HttpStatusCode.Conflict, "INVALID_STATE_TRANSITION");
    }
}
