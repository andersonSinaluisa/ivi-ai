using System.Text.Json;
using System.Reflection;
using System.Text.RegularExpressions;
using Jarvis.Api.Http;
using Jarvis.Application.Auth;
using Jarvis.Application.Common;
using Jarvis.Application.Realtime;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Jarvis.Domain.Events;
using Jarvis.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Tests;

public static class RepoPaths
{
    public static string Root { get; } = FindRoot();

    private static string FindRoot()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null && !File.Exists(Path.Combine(directory.FullName, "Jarvis.slnx")))
        {
            directory = directory.Parent;
        }

        return directory?.FullName ?? throw new InvalidOperationException("Repository root not found.");
    }
}

/// <summary>HU-001: reglas de dependencia de docs/01-architecture/backend-architecture.md.</summary>
public sealed class ArchitectureTests
{
    private static IReadOnlyList<string> References(Assembly assembly) =>
        assembly.GetReferencedAssemblies().Select(a => a.Name!).Where(n => n.StartsWith("Jarvis.", StringComparison.Ordinal) || n == "jarvis-api").ToList();

    [Fact]
    public void Domain_has_no_project_dependencies() =>
        Assert.Empty(References(typeof(Domain.Tasks.JarvisTask).Assembly));

    [Fact]
    public void Contracts_has_no_project_dependencies() =>
        Assert.Empty(References(typeof(MqttEnvelope).Assembly));

    [Fact]
    public void Application_depends_only_on_domain_and_contracts()
    {
        var references = References(typeof(Result<>).Assembly);
        Assert.All(references, r => Assert.Contains(r, new[] { "Jarvis.Domain", "Jarvis.Contracts" }));
    }

    [Fact]
    public void Infrastructure_does_not_depend_on_api() =>
        Assert.DoesNotContain("jarvis-api", References(typeof(JarvisDbContext).Assembly));

    [Fact]
    public void Simulators_depend_only_on_contracts()
    {
        var references = References(typeof(Simulators.DeviceConnection).Assembly);
        Assert.Equal(["Jarvis.Contracts"], references);
    }

    [Fact]
    public void Cloud_code_has_no_llm_provider_concepts()
    {
        // ADR-010: el Cloud no conoce proveedores, modelos ni prompts.
        var forbidden = new Regex(@"\b(claude|anthropic|openai|gpt-\d|prompt|llm)\b", RegexOptions.IgnoreCase);
        var offenders = Directory.EnumerateFiles(Path.Combine(RepoPaths.Root, "src"), "*.cs", SearchOption.AllDirectories)
            .Where(f => !f.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .Where(f => forbidden.IsMatch(File.ReadAllText(f)))
            .ToList();
        Assert.Empty(offenders);
    }

    [Fact]
    public void No_generic_execution_endpoint_exists()
    {
        var endpoints = File.ReadAllText(Path.Combine(RepoPaths.Root, "src", "Jarvis.Api", "Endpoints", "Endpoints.cs"));
        Assert.DoesNotMatch(new Regex(@"""/(execute|shell|command|exec)", RegexOptions.IgnoreCase), endpoints);
    }
}

/// <summary>HU-047: los ejemplos del protocolo documentado se serializan/deserializan con Jarvis.Contracts.</summary>
public sealed class ProtocolDocumentationTests
{
    private static readonly Dictionary<string, Type> DataTypes = new()
    {
        [MessageTypes.TaskCreate] = typeof(TaskCreateData),
        [MessageTypes.RequestAccepted] = typeof(RequestAcceptedData),
        [MessageTypes.RequestRejected] = typeof(RequestRejectedData),
        [MessageTypes.TaskDispatch] = typeof(TaskDispatchData),
        [MessageTypes.TaskCompleted] = typeof(TaskCompletedData),
        [MessageTypes.TaskFailed] = typeof(TaskFailedData),
        [MessageTypes.TaskUpdateAck] = typeof(TaskUpdateAckData),
        [MessageTypes.TaskCancel] = typeof(TaskCancelData),
        [MessageTypes.TaskApprovalDecision] = typeof(TaskApprovalDecisionData),
        [MessageTypes.TaskUpdate] = typeof(TaskUpdateData),
        [MessageTypes.PresenceHello] = typeof(PresenceHelloData),
        [MessageTypes.PresenceOffline] = typeof(PresenceOfflineData),
    };

    public static TheoryData<string> Examples()
    {
        var doc = File.ReadAllText(Path.Combine(RepoPaths.Root, "docs", "03-protocols", "mqtt-protocol.md"));
        var data = new TheoryData<string>();
        foreach (Match match in Regex.Matches(doc, "```json\\s*\\n(.*?)```", RegexOptions.Singleline))
        {
            var json = match.Groups[1].Value;
            if (json.Contains("\"v\": 1", StringComparison.Ordinal) && !json.Contains('…', StringComparison.Ordinal))
            {
                data.Add(json);
            }
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(Examples))]
    public void Documented_example_round_trips(string json)
    {
        var envelope = MqttEnvelope.Parse(System.Text.Encoding.UTF8.GetBytes(json));
        Assert.Equal(1, envelope.V);
        Assert.NotNull(envelope.Id);
        Assert.NotNull(envelope.DeviceId);
        Assert.True(DataTypes.TryGetValue(envelope.Type!, out var type), $"No data type mapped for {envelope.Type}");
        var data = envelope.Data.Deserialize(type, JarvisJson.Options);
        Assert.NotNull(data);

        var again = MqttEnvelope.Parse(envelope.ToBytes());
        Assert.Equal(envelope.Id, again.Id);
        Assert.Equal(envelope.Type, again.Type);
    }

    [Fact]
    public void Examples_were_found() => Assert.True(Examples().Count >= 9);

    [Theory]
    [InlineData("jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/up/tasks", true, "up", "tasks")]
    [InlineData("jarvis/v1/devices/0192A6F0-2222-7AAA-8BBB-000000000002/up/tasks", false, "", "")]
    [InlineData("jarvis/v1/devices/not-a-guid/up/tasks", false, "", "")]
    [InlineData("jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/sideways/tasks", false, "sideways", "tasks")]
    [InlineData("jarvis/v2/devices/0192a6f0-2222-7aaa-8bbb-000000000002/up/tasks", false, "", "")]
    public void Topic_parsing(string topic, bool valid, string direction, string channel)
    {
        Assert.Equal(valid, MqttTopics.TryParse(topic, out _, out var parsedDirection, out var parsedChannel));
        if (valid)
        {
            Assert.Equal(direction, parsedDirection);
            Assert.Equal(channel, parsedChannel);
        }
    }

    [Theory]
    [InlineData(MessageTypes.TaskDispatch, "tasks")]
    [InlineData(MessageTypes.TaskCancel, "tasks")]
    [InlineData(MessageTypes.TaskUpdateAck, "tasks")]
    [InlineData(MessageTypes.RequestAccepted, "notifications")]
    [InlineData(MessageTypes.TaskUpdate, "notifications")]
    [InlineData(MessageTypes.PresenceHelloAck, "control")]
    public void Down_channel_by_message_type(string type, string channel) => Assert.Equal(channel, MqttTopics.ChannelFor(type));
}

/// <summary>HU-003: el modelo EF es válido y la migración está al día con él.</summary>
public sealed class PersistenceModelTests
{
    private static JarvisDbContext CreateContext() =>
        new(new DbContextOptionsBuilder<JarvisDbContext>().UseNpgsql("Host=unused;Database=unused").Options);

    [Fact]
    public void Model_uses_snake_case_and_schema()
    {
        using var db = CreateContext();
        var tasks = db.Model.FindEntityType(typeof(Domain.Tasks.JarvisTask))!;
        Assert.Equal("tasks", tasks.GetTableName());
        Assert.Equal("jarvis", tasks.GetSchema());
        Assert.Equal("target_device_id", tasks.FindProperty(nameof(Domain.Tasks.JarvisTask.TargetDeviceId))!.GetColumnName());
        Assert.Equal("idempotency_key", tasks.FindProperty(nameof(Domain.Tasks.JarvisTask.IdempotencyKey))!.GetColumnName());
        Assert.Contains(tasks.GetIndexes(), i => i.GetDatabaseName() == "ux_tasks_idempotency" && i.IsUnique);
    }

    [Fact]
    public void Migrations_match_the_model()
    {
        using var db = CreateContext();
        Assert.False(db.Database.HasPendingModelChanges(), "Run 'dotnet ef migrations add' — the model changed.");
    }

    [Theory]
    [InlineData("TargetDeviceId", "target_device_id")]
    [InlineData("CreatedAt", "created_at")]
    [InlineData("Seq", "seq")]
    [InlineData("xmin", "xmin")]
    public void Snake_case(string input, string expected) => Assert.Equal(expected, SnakeCaseNaming.ToSnake(input));
}

public sealed class ApplicationHelpersTests
{
    [Theory]
    [InlineData("short", false)]
    [InlineData("password1234", false)]
    [InlineData("Correct-Horse-Battery-Staple-42", true)]
    [InlineData("a unique enough sentence", true)]
    public void Password_policy(string password, bool valid) => Assert.Equal(valid, PasswordPolicy.Validate(password) is null);

    [Fact]
    public void Keyset_cursor_round_trips()
    {
        var at = new DateTimeOffset(2026, 9, 19, 15, 0, 0, 123, TimeSpan.Zero);
        var id = Guid.CreateVersion7();
        Assert.True(KeysetCursor.TryDecode(KeysetCursor.Encode(at, id), out var decodedAt, out var decodedId));
        Assert.Equal(at, decodedAt);
        Assert.Equal(id, decodedId);
        Assert.False(KeysetCursor.TryDecode("garbage!", out _, out _));
    }

    [Fact]
    public void Device_secrets_have_prefix_and_256_bits()
    {
        var secret = SecretGenerator.NewDeviceSecret();
        Assert.StartsWith("jdv_", secret, StringComparison.Ordinal);
        Assert.Equal(47, secret.Length);
        Assert.NotEqual(secret, SecretGenerator.NewDeviceSecret());
    }

    [Theory]
    [InlineData(EventTypes.TaskCompleted, "task.completed")]
    [InlineData(EventTypes.DeviceConnected, "device.online")]
    [InlineData(EventTypes.DeviceDisconnected, "device.offline")]
    [InlineData(EventTypes.ApprovalDenied, "approval.resolved")]
    [InlineData(EventTypes.DeviceDisabled, "device.updated")]
    public void Realtime_mapping(string eventType, string notification)
    {
        var record = new EventRecord { Seq = 7, Type = eventType, Data = """{"code":"X","reason":"lwt"}""", OwnerId = Guid.NewGuid() };
        var mapped = RealtimeMapping.FromEvent(record);
        Assert.NotNull(mapped);
        Assert.Equal(notification, mapped.Name);
        Assert.Equal(7, mapped.Seq);
    }

    [Fact]
    public void Security_events_are_not_sent_to_realtime() =>
        Assert.Null(RealtimeMapping.FromEvent(new EventRecord { Type = EventTypes.SecurityViolation, Data = "{}" }));

    [Theory]
    [InlineData(ErrorCodes.ValidationFailed, 400)]
    [InlineData(ErrorCodes.InvalidCredentials, 401)]
    [InlineData(ErrorCodes.TaskNotGranted, 403)]
    [InlineData(ErrorCodes.NotFound, 404)]
    [InlineData(ErrorCodes.InvalidStateTransition, 409)]
    [InlineData(ErrorCodes.PayloadTooLarge, 413)]
    [InlineData(ErrorCodes.IdempotencyKeyReused, 422)]
    [InlineData(ErrorCodes.AccountLocked, 423)]
    [InlineData(ErrorCodes.RateLimited, 429)]
    [InlineData(ErrorCodes.ServiceUnavailable, 503)]
    public void Error_codes_map_to_http_status(string code, int status) => Assert.Equal(status, Problems.StatusFor(code));

    [Fact]
    public void Device_rate_limits_drop_excess_and_detect_flood()
    {
        using var limits = new Mqtt.DeviceRateLimits(
            Microsoft.Extensions.Options.Options.Create(new Configuration.SecurityOptions { MqttMessagesPerSecond = 3, FloodViolationAfterSeconds = 60 }),
            Microsoft.Extensions.Options.Options.Create(new Configuration.TaskOptions { CreateLimitPerMinute = 2 }));
        var device = Guid.NewGuid();
        Assert.Equal(3, Enumerable.Range(0, 10).Count(_ => limits.TryAcquireMessage(device)));
        Assert.True(limits.TryAcquireTaskCreate(device));
        Assert.True(limits.TryAcquireTaskCreate(device));
        Assert.False(limits.TryAcquireTaskCreate(device));

        var t0 = DateTimeOffset.UtcNow;
        Assert.False(limits.RegisterRejection(device, t0));
        for (var s = 1; s < 60; s++)
        {
            Assert.False(limits.RegisterRejection(device, t0.AddSeconds(s)));
        }

        Assert.True(limits.RegisterRejection(device, t0.AddSeconds(61)));
        Assert.False(limits.RegisterRejection(device, t0.AddSeconds(62)));
    }
}
