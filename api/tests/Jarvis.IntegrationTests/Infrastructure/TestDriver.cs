using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;
using System.Text.Json.Serialization;
using Jarvis.Application.Auth;
using Jarvis.Contracts.Rest;
using Jarvis.Simulators;
using Microsoft.Extensions.DependencyInjection;

namespace Jarvis.IntegrationTests.Infrastructure;

[CollectionDefinition(Name)]
public sealed class JarvisCollection : ICollectionFixture<JarvisTestEnvironment>
{
    public const string Name = "jarvis";
}

/// <summary>Owner autenticado con helpers de alto nivel sobre la REST API real.</summary>
public sealed class OwnerSession(JarvisTestEnvironment env, HttpClient client, Guid userId, string email, string password, LoginResponse login)
{
    public static readonly JsonSerializerOptions Json = CreateJson();

    public HttpClient Client { get; private set; } = client;

    public Guid UserId { get; } = userId;

    public string Email { get; } = email;

    public string Password { get; } = password;

    public LoginResponse Login { get; } = login;

    public JarvisTestEnvironment Environment { get; } = env;

    /// <summary>Tras reiniciar la API (nuevo TestServer), crea un cliente nuevo con el mismo token.</summary>
    public void RefreshClient()
    {
        var client = Environment.CreateClient();
        client.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", Login.AccessToken);
        Client = client;
    }

    public static async Task<OwnerSession> CreateAsync(JarvisTestEnvironment env)
    {
        var email = $"owner-{Guid.NewGuid():N}@example.com";
        var password = "Owner-Test-Passphrase-" + Guid.NewGuid().ToString("N")[..8];
        Guid userId;
        await using (var scope = env.Services.CreateAsyncScope())
        {
            var result = await scope.ServiceProvider.GetRequiredService<BootstrapOwnerHandler>()
                .HandleAsync(email, "Test Owner", password, CancellationToken.None);
            Assert.True(result.IsSuccess, result.Error?.Message);
            userId = result.Value;
        }

        var client = env.CreateClient();
        var response = await client.PostAsJsonAsync("/api/v1/auth/login", new LoginRequest(email, password));
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var login = (await response.Content.ReadFromJsonAsync<LoginResponse>(Json))!;
        client.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", login.AccessToken);
        return new OwnerSession(env, client, userId, email, password, login);
    }

    public async Task<RegisterDeviceResponse> RegisterDeviceAsync(string type, string? name = null)
    {
        var response = await Client.PostAsJsonAsync("/api/v1/devices", new CreateDeviceRequest(name ?? $"{type}-{Guid.NewGuid():N}"[..20], type));
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await response.Content.ReadFromJsonAsync<RegisterDeviceResponse>(Json))!;
    }

    public async Task<GrantDto> GrantAsync(Guid source, Guid target, string taskType = "OPEN_CHROME")
    {
        var response = await Client.PostAsJsonAsync($"/api/v1/devices/{source}/grants", new CreateGrantRequest(target, taskType));
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await response.Content.ReadFromJsonAsync<GrantDto>(Json))!;
    }

    public async Task<DeviceDto> GetDeviceAsync(Guid id) =>
        (await Client.GetFromJsonAsync<DeviceDto>($"/api/v1/devices/{id}", Json))!;

    public async Task<TaskDto> GetTaskAsync(Guid id) =>
        (await Client.GetFromJsonAsync<TaskDto>($"/api/v1/tasks/{id}", Json))!;

    public async Task<EventsPage> EventsAsync(string query) =>
        (await Client.GetFromJsonAsync<EventsPage>($"/api/v1/events?{query}", Json))!;

    public async Task<TaskDto> WaitForTaskStatusAsync(Guid taskId, string status, TimeSpan? timeout = null) =>
        await JarvisTestEnvironment.WaitForAsync(
            async () =>
            {
                var task = await GetTaskAsync(taskId);
                return task.Status == status ? task : null;
            },
            timeout ?? TimeSpan.FromSeconds(20),
            $"task {taskId} to be {status}");

    public async Task<DeviceDto> WaitForDeviceStatusAsync(Guid deviceId, string status, TimeSpan? timeout = null) =>
        await JarvisTestEnvironment.WaitForAsync(
            async () =>
            {
                var device = await GetDeviceAsync(deviceId);
                return device.Status == status ? device : null;
            },
            timeout ?? TimeSpan.FromSeconds(20),
            $"device {deviceId} to be {status}");

    public DeviceSettings DeviceSettings(IssuedCredentialDto credential) => new()
    {
        DeviceId = credential.DeviceId,
        Secret = credential.Secret,
        Host = Environment.MqttHost,
        Port = Environment.MqttTlsPort,
        UseTls = true,
        CaFile = Environment.CaFile,
        AuthFailureBackoff = TimeSpan.FromSeconds(2),
    };

    public WindowsSimulator StartWindows(IssuedCredentialDto credential, WindowsSimulatorOptions? options = null, ITestOutputHelper? output = null)
    {
        var simulator = new WindowsSimulator(
            DeviceSettings(credential),
            Environment.StateDirectory,
            options ?? new WindowsSimulatorOptions { ExecutionDelay = TimeSpan.FromMilliseconds(300), OutboxResendInterval = TimeSpan.FromSeconds(3) },
            m => output?.WriteLine(m));
        simulator.Start();
        return simulator;
    }

    public Esp32Simulator StartEsp32(IssuedCredentialDto credential, Guid target, ITestOutputHelper? output = null, int expiresInSeconds = 300)
    {
        var simulator = new Esp32Simulator(
            DeviceSettings(credential),
            Environment.StateDirectory,
            new Esp32SimulatorOptions { TargetDeviceId = target, ExpiresInSeconds = expiresInSeconds },
            m => output?.WriteLine(m));
        simulator.Start();
        return simulator;
    }

    public async Task<TaskDto> CreateTaskAsync(Guid target, string type = "OPEN_CHROME", object? payload = null, int? expiresInSeconds = null, int? maxDurationSeconds = null, string? idempotencyKey = null)
    {
        var response = await PostTaskAsync(Client, target, type, payload, expiresInSeconds, maxDurationSeconds, idempotencyKey);
        Assert.True(response.StatusCode is HttpStatusCode.Created or HttpStatusCode.OK, await response.Content.ReadAsStringAsync());
        return (await response.Content.ReadFromJsonAsync<TaskDto>(Json))!;
    }

    public static Task<HttpResponseMessage> PostTaskAsync(
        HttpClient client,
        Guid target,
        string type = "OPEN_CHROME",
        object? payload = null,
        int? expiresInSeconds = null,
        int? maxDurationSeconds = null,
        string? idempotencyKey = null)
    {
        var request = new HttpRequestMessage(HttpMethod.Post, "/api/v1/tasks")
        {
            Content = JsonContent.Create(new
            {
                targetDeviceId = target,
                type,
                payload = payload ?? new { url = "https://example.com" },
                expiresInSeconds,
                maxDurationSeconds,
            }),
        };
        if (idempotencyKey is not null)
        {
            request.Headers.Add("Idempotency-Key", idempotencyKey);
        }

        return client.SendAsync(request);
    }

    /// <summary>Cliente autenticado como device (token de device vía REST).</summary>
    public async Task<HttpClient> DeviceClientAsync(IssuedCredentialDto credential)
    {
        var client = Environment.CreateClient();
        var response = await client.PostAsJsonAsync("/api/v1/auth/device-token", new DeviceTokenRequest(credential.DeviceId, credential.Secret));
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var token = (await response.Content.ReadFromJsonAsync<AccessTokenResponse>(Json))!;
        client.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", token.AccessToken);
        return client;
    }

    private static JsonSerializerOptions CreateJson()
    {
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web);
        options.Converters.Add(new JsonStringEnumConverter());
        return options;
    }
}

public static class ProblemAssertions
{
    public static async Task AssertProblemAsync(this HttpResponseMessage response, HttpStatusCode status, string code)
    {
        var body = await response.Content.ReadAsStringAsync();
        Assert.True(response.StatusCode == status, $"Expected {status} but got {response.StatusCode}: {body}");
        Assert.Equal("application/problem+json", response.Content.Headers.ContentType?.MediaType);
        using var document = JsonDocument.Parse(body);
        Assert.Equal(code, document.RootElement.GetProperty("code").GetString());
        Assert.True(document.RootElement.TryGetProperty("correlationId", out _));
    }
}
