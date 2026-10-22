using System.Collections.Concurrent;
using System.Net;
using System.Net.Http.Json;
using Jarvis.Contracts.Mqtt;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Events;
using Jarvis.IntegrationTests.Infrastructure;
using Jarvis.Simulators;

namespace Jarvis.E2ETests;

[CollectionDefinition(Name)]
public sealed class E2ECollection : ICollectionFixture<JarvisTestEnvironment>
{
    public const string Name = "e2e";
}

/// <summary>
/// Escenarios E2E-01 … E2E-10 de docs/10-testing/integration-scenarios.md: API real, PostgreSQL y Mosquitto reales
/// (TLS en 8883 con CA de prueba) y simuladores que implementan el protocolo completo.
/// </summary>
[Collection(E2ECollection.Name)]
public sealed class MvpScenarios(JarvisTestEnvironment env, ITestOutputHelper output)
{
    private static readonly TimeSpan Wait = TimeSpan.FromSeconds(30);

    [Fact]
    public async Task E2E_01_esp32_button_opens_chrome_on_windows_and_result_returns()
    {
        var (owner, esp, pc) = await SetupAsync();
        await using var windows = owner.StartWindows(pc.Credential, output: output);
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output);
        var updates = Track(esp32);
        await windows.Connection.WaitReadyAsync(Wait);
        await esp32.Connection.WaitReadyAsync(Wait);
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");
        await owner.WaitForDeviceStatusAsync(esp.Device.Id, "Online");

        var press = await esp32.PressButtonAsync();
        Assert.True(press.Accepted, press.ErrorCode);
        var taskId = press.TaskId!.Value;

        await JarvisTestEnvironment.WaitUntilAsync(() => Final(updates, taskId) is not null, Wait, "ESP32 to receive the final update");
        var final = Final(updates, taskId)!;
        Assert.Equal("Completed", final.Status);
        Assert.Equal("Chrome opened (simulated)", final.Result!.Value.GetProperty("message").GetString());
        Assert.Contains(updates[taskId], u => u.Status == "Running");

        var task = await owner.GetTaskAsync(taskId);
        Assert.Equal("Completed", task.Status);
        Assert.Equal("Device", task.RequestedBy.Type);
        Assert.Equal(esp.Device.Id, task.SourceDeviceId);
        Assert.Equal(pc.Device.Id, task.TargetDeviceId);
        Assert.NotNull(task.DispatchedAt);
        Assert.NotNull(task.DeliveredAt);
        Assert.NotNull(task.StartedAt);
        Assert.NotNull(task.CompletedAt);
        Assert.Equal(1, windows.ExecutionCount(taskId));

        var history = await owner.EventsAsync($"correlationId={task.CorrelationId}&after=0");
        Assert.Equal(
            [EventTypes.TaskCreated, EventTypes.TaskDispatched, EventTypes.TaskDelivered, EventTypes.TaskStarted, EventTypes.TaskCompleted],
            history.Items.Select(e => e.Type));
        Assert.All(history.Items, e => Assert.Equal(task.CorrelationId, e.CorrelationId));
        await JarvisTestEnvironment.WaitUntilAsync(() => windows.IsSettled(taskId), Wait, "windows to settle the task");
    }

    [Fact]
    public async Task E2E_02_task_waits_for_offline_target()
    {
        var (owner, esp, pc) = await SetupAsync();
        await ReportCapabilitiesThenStopAsync(owner, pc);
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output, expiresInSeconds: 120);
        await esp32.Connection.WaitReadyAsync(Wait);

        var press = await esp32.PressButtonAsync();
        Assert.True(press.Accepted, press.ErrorCode);
        await Task.Delay(2000);
        var pending = await owner.GetTaskAsync(press.TaskId!.Value);
        Assert.Equal("Pending", pending.Status);
        Assert.Equal(0, pending.AttemptCount);

        await using var windows = owner.StartWindows(pc.Credential, output: output);
        await owner.WaitForTaskStatusAsync(press.TaskId.Value, "Completed", Wait);
        Assert.Equal(1, windows.ExecutionCount(press.TaskId.Value));
    }

    [Fact]
    public async Task E2E_03_lost_acks_are_redelivered_without_double_execution()
    {
        var (owner, _, pc) = await SetupAsync();
        await using var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { DropAcks = 2, ExecutionDelay = TimeSpan.FromMilliseconds(200) }, output);
        await windows.Connection.WaitReadyAsync(Wait);
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");

        var task = await owner.CreateTaskAsync(pc.Device.Id);
        var completed = await owner.WaitForTaskStatusAsync(task.Id, "Completed", TimeSpan.FromSeconds(45));
        Assert.Equal(3, completed.AttemptCount);
        Assert.Equal(1, windows.ExecutionCount(task.Id));
        var dispatches = await owner.EventsAsync($"taskId={task.Id}&type={EventTypes.TaskDispatched}&after=0");
        Assert.Equal([1, 2, 3], dispatches.Items.Select(e => e.Data.GetProperty("attempt").GetInt32()));
    }

    [Fact]
    public async Task E2E_04_connection_lost_before_ack_executes_once()
    {
        var (owner, _, pc) = await SetupAsync();
        await using var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { DisconnectBeforeAck = true, ExecutionDelay = TimeSpan.FromMilliseconds(200) }, output);
        await windows.Connection.WaitReadyAsync(Wait);
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");

        var task = await owner.CreateTaskAsync(pc.Device.Id);
        await owner.WaitForTaskStatusAsync(task.Id, "Completed", Wait);
        Assert.Equal(1, windows.ExecutionCount(task.Id));

        // El LWT de la sesión anterior no deja el device Offline.
        await Task.Delay(1500);
        Assert.Equal("Online", (await owner.GetDeviceAsync(pc.Device.Id)).PresenceState);
    }

    [Fact]
    public async Task E2E_05_authorization_is_enforced_end_to_end()
    {
        var (owner, esp, pc) = await SetupAsync(grant: false);
        await ReportCapabilitiesThenStopAsync(owner, pc);
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output);
        await esp32.Connection.WaitReadyAsync(Wait);

        var denied = await esp32.PressButtonAsync();
        Assert.False(denied.Accepted);
        Assert.Equal("TASK_NOT_GRANTED", denied.ErrorCode);
        Assert.NotEmpty((await owner.EventsAsync($"type={EventTypes.TaskRejected}")).Items);

        Assert.False(await esp32.Connection.TryPublishRawAsync(MqttTopics.Up(pc.Device.Id, TopicChannels.Tasks), "{}"u8.ToArray()));
        Assert.False(await esp32.Connection.TrySubscribeRawAsync(MqttTopics.DeviceSubscription(pc.Device.Id)));

        var attacker = await OwnerSession.CreateAsync(env);
        Assert.Equal(HttpStatusCode.NotFound, (await attacker.Client.GetAsync($"/api/v1/devices/{pc.Device.Id}")).StatusCode);
    }

    [Fact]
    public async Task E2E_06_undelivered_task_expires_and_origin_is_notified()
    {
        var (owner, esp, pc) = await SetupAsync();
        await ReportCapabilitiesThenStopAsync(owner, pc);
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output, expiresInSeconds: 10);
        var updates = Track(esp32);
        await esp32.Connection.WaitReadyAsync(Wait);

        var press = await esp32.PressButtonAsync();
        var taskId = press.TaskId!.Value;
        var expired = await owner.WaitForTaskStatusAsync(taskId, "Expired", Wait);
        Assert.Equal("NOT_DELIVERED", expired.Error!.Code);
        await JarvisTestEnvironment.WaitUntilAsync(() => Final(updates, taskId)?.Status == "Expired", Wait, "ESP32 to see Expired");

        await using var windows = owner.StartWindows(pc.Credential, output: output);
        await windows.Connection.WaitReadyAsync(Wait);
        await Task.Delay(2000);
        Assert.Equal(0, windows.ExecutionCount(taskId));
    }

    [Fact]
    public async Task E2E_07_revocation_disconnects_and_cancels_running_tasks()
    {
        var (owner, esp, pc) = await SetupAsync();
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output);
        await using var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { ExecutionDelay = TimeSpan.FromSeconds(30) }, output);
        await esp32.Connection.WaitReadyAsync(Wait);
        await windows.Connection.WaitReadyAsync(Wait);

        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{esp.Device.Id}/revoke", null)).StatusCode);
        await JarvisTestEnvironment.WaitUntilAsync(() => !esp32.Connection.IsConnected, TimeSpan.FromSeconds(5), "revoked ESP32 to be disconnected");
        await Task.Delay(3000);
        Assert.False(esp32.Connection.IsConnected);
        var grants = await owner.Client.GetFromJsonAsync<System.Text.Json.JsonElement>($"/api/v1/devices/{esp.Device.Id}/grants", OwnerSession.Json);
        Assert.Equal(HttpStatusCode.Conflict, (await owner.Client.PostAsJsonAsync($"/api/v1/devices/{esp.Device.Id}/grants", new CreateGrantRequest(pc.Device.Id, "OPEN_CHROME"))).StatusCode);
        Assert.Equal(0, grants.GetProperty("items").GetArrayLength());

        var task = await owner.CreateTaskAsync(pc.Device.Id);
        await owner.WaitForTaskStatusAsync(task.Id, "Running", Wait);
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{pc.Device.Id}/revoke", null)).StatusCode);
        var cancelled = await owner.GetTaskAsync(task.Id);
        Assert.Equal("Cancelled", cancelled.Status);
        Assert.Equal("DEVICE_REVOKED", cancelled.Error!.Code);
    }

    [Fact]
    public async Task E2E_08_presence_online_stale_offline_busy_disabled()
    {
        var (owner, _, pc) = await SetupAsync();
        var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { Busy = true }, output);
        try
        {
            await windows.Connection.WaitReadyAsync(Wait);
            await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Busy");

            windows.Connection.PauseHeartbeats = true;
            await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Stale");
            var offline = await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Offline");
            Assert.Equal("Offline", offline.PresenceState);
            windows.Connection.PauseHeartbeats = false;
            await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Busy");
            var disconnects = await owner.EventsAsync($"deviceId={pc.Device.Id}&type={EventTypes.DeviceDisconnected}");
            Assert.Contains(disconnects.Items, e => e.Data.GetProperty("reason").GetString() == "heartbeat_timeout");

            await windows.Connection.CrashAsync();
            await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Offline", TimeSpan.FromSeconds(10));
            var lwt = await owner.EventsAsync($"deviceId={pc.Device.Id}&type={EventTypes.DeviceDisconnected}");
            Assert.Contains(lwt.Items, e => e.Data.GetProperty("reason").GetString() == "lwt");
        }
        finally
        {
            await windows.DisposeAsync();
        }

        await using var again = owner.StartWindows(pc.Credential, output: output);
        await again.Connection.WaitReadyAsync(Wait);
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Online");
        Assert.Equal(HttpStatusCode.OK, (await owner.Client.PostAsync($"/api/v1/devices/{pc.Device.Id}/disable", null)).StatusCode);
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Disabled");
        await JarvisTestEnvironment.WaitUntilAsync(() => !again.Connection.IsConnected, TimeSpan.FromSeconds(5), "disabled device disconnected");
    }

    [Fact]
    public async Task E2E_09_cancellation_of_running_and_pending_tasks()
    {
        var (owner, esp, pc) = await SetupAsync();
        await using var esp32 = owner.StartEsp32(esp.Credential, pc.Device.Id, output);
        var updates = Track(esp32);
        await esp32.Connection.WaitReadyAsync(Wait);
        await using (var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { ExecutionDelay = TimeSpan.FromSeconds(30) }, output))
        {
            await windows.Connection.WaitReadyAsync(Wait);
            var press = await esp32.PressButtonAsync();
            var taskId = press.TaskId!.Value;
            await owner.WaitForTaskStatusAsync(taskId, "Running", Wait);

            var cancel = await owner.Client.PostAsync($"/api/v1/tasks/{taskId}/cancel", null);
            Assert.Equal(HttpStatusCode.Accepted, cancel.StatusCode);
            await owner.WaitForTaskStatusAsync(taskId, "Cancelled", Wait);
            await JarvisTestEnvironment.WaitUntilAsync(() => Final(updates, taskId)?.Status == "Cancelled", Wait, "ESP32 to see Cancelled");
            await windows.StopAsync();
        }

        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Offline");
        var pending = await owner.CreateTaskAsync(pc.Device.Id);
        var immediate = await owner.Client.PostAsync($"/api/v1/tasks/{pending.Id}/cancel", null);
        Assert.Equal(HttpStatusCode.OK, immediate.StatusCode);

        await using var restarted = owner.StartWindows(pc.Credential, output: output);
        await restarted.Connection.WaitReadyAsync(Wait);
        await Task.Delay(2000);
        Assert.Equal(0, restarted.ExecutionCount(pending.Id));
    }

    [Fact]
    public async Task E2E_10_tasks_survive_api_and_broker_restarts()
    {
        var (owner, _, pc) = await SetupAsync();
        await ReportCapabilitiesThenStopAsync(owner, pc);
        var task = await owner.CreateTaskAsync(pc.Device.Id, expiresInSeconds: 300);

        await env.StopApiAsync();
        await env.RestartMosquittoAsync();
        await env.StartApiAsync();
        owner.RefreshClient();
        Assert.Equal("Pending", (await owner.GetTaskAsync(task.Id)).Status);

        await using (var windows = owner.StartWindows(pc.Credential, new WindowsSimulatorOptions { ExecutionDelay = TimeSpan.FromSeconds(4) }, output))
        {
            await owner.WaitForTaskStatusAsync(task.Id, "Completed", Wait);
            Assert.Equal(1, windows.ExecutionCount(task.Id));

            // Variante: el device termina mientras la API está detenida; el resultado espera en la sesión persistente.
            var second = await owner.CreateTaskAsync(pc.Device.Id);
            await owner.WaitForTaskStatusAsync(second.Id, "Running", Wait);
            await env.StopApiAsync();
            await JarvisTestEnvironment.WaitUntilAsync(() => windows.LocalStatus(second.Id) == "Completed", Wait, "windows to finish while API is down");
            await env.StartApiAsync();
            owner.RefreshClient();
            await owner.WaitForTaskStatusAsync(second.Id, "Completed", Wait);
            Assert.Equal(1, windows.ExecutionCount(second.Id));

            var events = await owner.EventsAsync($"taskId={second.Id}&after=0");
            Assert.Single(events.Items, e => e.Type == EventTypes.TaskCompleted);
        }
    }

    private async Task<(OwnerSession Owner, RegisterDeviceResponse Esp, RegisterDeviceResponse Pc)> SetupAsync(bool grant = true)
    {
        Assert.SkipUnless(env.Available, env.UnavailableReason ?? "Docker unavailable");
        var owner = await OwnerSession.CreateAsync(env);
        var esp = await owner.RegisterDeviceAsync("ESP32");
        var pc = await owner.RegisterDeviceAsync("Windows");
        if (grant)
        {
            await owner.GrantAsync(esp.Device.Id, pc.Device.Id);
        }

        return (owner, esp, pc);
    }

    /// <summary>El target debe haber enviado hello (supportedTaskTypes) para aceptar tareas.</summary>
    private async Task ReportCapabilitiesThenStopAsync(OwnerSession owner, RegisterDeviceResponse pc)
    {
        await using var windows = owner.StartWindows(pc.Credential, output: output);
        await windows.Connection.WaitReadyAsync(Wait);
        await windows.StopAsync();
        await owner.WaitForDeviceStatusAsync(pc.Device.Id, "Offline");
    }

    private static ConcurrentDictionary<Guid, List<TaskUpdateData>> Track(Esp32Simulator esp32)
    {
        var updates = new ConcurrentDictionary<Guid, List<TaskUpdateData>>();
        esp32.TaskUpdated += update =>
        {
            var list = updates.GetOrAdd(update.TaskId, _ => []);
            lock (list)
            {
                list.Add(update);
            }
        };
        return updates;
    }

    private static TaskUpdateData? Final(ConcurrentDictionary<Guid, List<TaskUpdateData>> updates, Guid taskId)
    {
        if (!updates.TryGetValue(taskId, out var list))
        {
            return null;
        }

        lock (list)
        {
            return list.LastOrDefault(u => u.Status is "Completed" or "Failed" or "Cancelled" or "Expired");
        }
    }
}
