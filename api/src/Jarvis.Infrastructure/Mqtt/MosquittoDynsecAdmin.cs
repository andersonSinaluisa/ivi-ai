using System.Buffers;
using System.Collections.Concurrent;
using System.Text.Json;
using System.Text.Json.Nodes;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Configuration;
using Jarvis.Contracts.Mqtt;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using MQTTnet;
using MQTTnet.Formatter;
using MQTTnet.Protocol;

namespace Jarvis.Infrastructure.Mqtt;

/// <summary>Comandos de Mosquitto Dynamic Security (<c>$CONTROL/dynamic-security/v1</c>).</summary>
public static class DynsecCommands
{
    public const string CloudRole = "jarvis-cloud";
    public const string DeviceRolePrefix = "jarvis-device-";

    public static string DeviceRole(Guid deviceId) => DeviceRolePrefix + deviceId.ToString("D");

    /// <summary>
    /// Rol por device con topics literales (fallback documentado en ADR-003): no depende de la sustitución <c>%u</c>.
    /// </summary>
    public static JsonObject CreateDeviceRole(Guid deviceId)
    {
        var id = deviceId.ToString("D");
        return new JsonObject
        {
            ["command"] = "createRole",
            ["rolename"] = DeviceRole(deviceId),
            ["acls"] = new JsonArray(
                Acl("publishClientSend", $"{MqttTopics.Prefix}{id}/up/#"),
                Acl("subscribeLiteral", $"{MqttTopics.Prefix}{id}/down/#"),
                Acl("subscribePattern", $"{MqttTopics.Prefix}{id}/down/#"),
                Acl("publishClientReceive", $"{MqttTopics.Prefix}{id}/down/#")),
        };
    }

    public static JsonObject CreateCloudRole() => new()
    {
        ["command"] = "createRole",
        ["rolename"] = CloudRole,
        ["acls"] = CloudAcls(),
    };

    public static JsonArray CloudAcls() => new(
        Acl("publishClientSend", "jarvis/v1/devices/+/down/#"),
        Acl("subscribePattern", MqttTopics.CloudSubscription),
        Acl("subscribeLiteral", MqttTopics.CloudSubscription),
        Acl("publishClientReceive", MqttTopics.CloudSubscription),
        Acl("publishClientSend", "$CONTROL/dynamic-security/#"),
        Acl("subscribePattern", "$CONTROL/dynamic-security/#"),
        Acl("subscribeLiteral", MqttCloudConnection.DynsecResponseTopic),
        Acl("publishClientReceive", "$CONTROL/dynamic-security/#"));

    public static JsonObject CreateClient(string username, string password, string? clientId, string role)
    {
        var command = new JsonObject
        {
            ["command"] = "createClient",
            ["username"] = username,
            ["password"] = password,
            ["roles"] = new JsonArray(new JsonObject { ["rolename"] = role, ["priority"] = -1 }),
        };
        if (clientId is not null)
        {
            // Fija el clientId: una conexión con otro clientId para este username es rechazada.
            command["clientid"] = clientId;
        }

        return command;
    }

    public static JsonObject DefaultDenyAcl() => new()
    {
        ["command"] = "setDefaultACLAccess",
        ["acls"] = new JsonArray(
            new JsonObject { ["acltype"] = "publishClientSend", ["allow"] = false },
            new JsonObject { ["acltype"] = "publishClientReceive", ["allow"] = false },
            new JsonObject { ["acltype"] = "subscribe", ["allow"] = false },
            new JsonObject { ["acltype"] = "unsubscribe", ["allow"] = true }),
    };

    public static JsonObject Simple(string command, string key, string value) => new() { ["command"] = command, [key] = value };

    private static JsonObject Acl(string type, string topic) =>
        new() { ["acltype"] = type, ["topic"] = topic, ["priority"] = 0, ["allow"] = true };
}

/// <summary>
/// HU-018: implementa <see cref="IBrokerCredentialAdmin"/> usando la conexión del cloud client.
/// Correlaciona comando/respuesta con <c>correlationData</c>; timeout configurable (5 s).
/// </summary>
public sealed class MosquittoDynsecAdmin : IBrokerCredentialAdmin
{
    private readonly MqttCloudConnection _connection;
    private readonly TimeSpan _timeout;
    private readonly ILogger<MosquittoDynsecAdmin> _logger;
    private readonly ConcurrentDictionary<string, TaskCompletionSource<JsonObject>> _pending = new();

    public MosquittoDynsecAdmin(MqttCloudConnection connection, IOptions<MqttOptions> options, ILogger<MosquittoDynsecAdmin> logger)
    {
        _connection = connection;
        _timeout = TimeSpan.FromSeconds(options.Value.AdminTimeoutSeconds);
        _logger = logger;
        _connection.DynsecResponseReceived += OnResponse;
    }

    public async Task CreateClientAsync(Guid deviceId, string secret, CancellationToken cancellationToken)
    {
        await ExecuteAsync(DynsecCommands.CreateDeviceRole(deviceId), cancellationToken, ignoreAlreadyExists: true);
        var username = deviceId.ToString("D");
        try
        {
            await ExecuteAsync(DynsecCommands.CreateClient(username, secret, username, DynsecCommands.DeviceRole(deviceId)), cancellationToken);
        }
        catch (BrokerCommandException exception) when (exception.IsAlreadyExists)
        {
            // Reintento tras un fallo previo: se alinea la contraseña con la nueva credencial.
            await SetPasswordAsync(deviceId, secret, cancellationToken);
        }
    }

    public Task SetPasswordAsync(Guid deviceId, string secret, CancellationToken cancellationToken) =>
        ExecuteAsync(new JsonObject { ["command"] = "setClientPassword", ["username"] = deviceId.ToString("D"), ["password"] = secret }, cancellationToken);

    public Task DisableClientAsync(Guid deviceId, CancellationToken cancellationToken) =>
        ExecuteAsync(DynsecCommands.Simple("disableClient", "username", deviceId.ToString("D")), cancellationToken);

    public Task EnableClientAsync(Guid deviceId, CancellationToken cancellationToken) =>
        ExecuteAsync(DynsecCommands.Simple("enableClient", "username", deviceId.ToString("D")), cancellationToken);

    public async Task DeleteClientAsync(Guid deviceId, CancellationToken cancellationToken)
    {
        await ExecuteAsync(DynsecCommands.Simple("deleteClient", "username", deviceId.ToString("D")), cancellationToken, ignoreNotFound: true);
        await ExecuteAsync(DynsecCommands.Simple("deleteRole", "rolename", DynsecCommands.DeviceRole(deviceId)), cancellationToken, ignoreNotFound: true);
    }

    public async Task<IReadOnlyList<BrokerClientInfo>> ListClientsAsync(CancellationToken cancellationToken)
    {
        var response = await ExecuteAsync(
            new JsonObject { ["command"] = "listClients", ["verbose"] = true, ["count"] = -1, ["offset"] = 0 },
            cancellationToken);
        var clients = response["data"]?["clients"]?.AsArray() ?? [];
        return clients
            .OfType<JsonObject>()
            .Select(c => new BrokerClientInfo(
                c["username"]?.GetValue<string>() ?? string.Empty,
                c["disabled"]?.GetValue<bool>() ?? false,
                (c["roles"]?.AsArray() ?? []).OfType<JsonObject>().Select(r => r["rolename"]?.GetValue<string>() ?? string.Empty).ToList()))
            .ToList();
    }

    public async Task<JsonObject> ExecuteAsync(
        JsonObject command,
        CancellationToken cancellationToken,
        bool ignoreAlreadyExists = false,
        bool ignoreNotFound = false)
    {
        if (!_connection.IsConnected)
        {
            throw new BrokerUnavailableException("MQTT cloud client is not connected.");
        }

        var name = command["command"]!.GetValue<string>();
        var correlation = Guid.NewGuid().ToString("N");
        command["correlationData"] = correlation;
        var completion = new TaskCompletionSource<JsonObject>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[correlation] = completion;
        try
        {
            var body = new JsonObject { ["commands"] = new JsonArray(command) };
            if (!await _connection.PublishAsync(MqttCloudConnection.DynsecCommandTopic, JsonSerializer.SerializeToUtf8Bytes(body), cancellationToken))
            {
                throw new BrokerUnavailableException($"Could not publish dynsec command {name}.");
            }

            JsonObject response;
            try
            {
                response = await completion.Task.WaitAsync(_timeout, cancellationToken);
            }
            catch (TimeoutException exception)
            {
                throw new BrokerUnavailableException($"Dynsec command {name} timed out.", exception);
            }

            if (response["error"]?.GetValue<string>() is { Length: > 0 } error)
            {
                var failure = new BrokerCommandException(name, error);
                if ((ignoreAlreadyExists && failure.IsAlreadyExists) || (ignoreNotFound && failure.IsNotFound))
                {
                    return response;
                }

                throw failure;
            }

            _logger.LogDebug("Dynsec command {Command} succeeded", name);
            return response;
        }
        finally
        {
            _pending.TryRemove(correlation, out _);
        }
    }

    private void OnResponse(ReadOnlyMemory<byte> payload)
    {
        try
        {
            var root = JsonNode.Parse(payload.Span)?.AsObject();
            foreach (var response in (root?["responses"]?.AsArray() ?? []).OfType<JsonObject>())
            {
                if (response["correlationData"]?.GetValue<string>() is { } correlation && _pending.TryGetValue(correlation, out var completion))
                {
                    completion.TrySetResult((JsonObject)response.DeepClone());
                }
            }
        }
        catch (JsonException exception)
        {
            _logger.LogWarning(exception, "Invalid dynsec response");
        }
    }
}

/// <summary>
/// Bootstrap idempotente de Dynamic Security (docs/05-security/secrets-management.md): roles, ACL por defecto deny
/// y client <c>jarvis-cloud</c>. Se ejecuta con el admin temporal creado por <c>mosquitto_ctrl dynsec init</c>.
/// </summary>
public sealed class DynsecBootstrapper(ILogger logger)
{
    public async Task RunAsync(
        string host,
        int port,
        string adminUsername,
        string adminPassword,
        string cloudPassword,
        bool deleteAdmin,
        CancellationToken cancellationToken)
    {
        using var client = new MqttClientFactory().CreateMqttClient();
        var responses = new ConcurrentDictionary<string, TaskCompletionSource<JsonObject>>();
        client.ApplicationMessageReceivedAsync += e =>
        {
            var root = JsonNode.Parse(e.ApplicationMessage.Payload.ToArray())?.AsObject();
            foreach (var response in (root?["responses"]?.AsArray() ?? []).OfType<JsonObject>())
            {
                if (response["correlationData"]?.GetValue<string>() is { } c && responses.TryGetValue(c, out var tcs))
                {
                    tcs.TrySetResult((JsonObject)response.DeepClone());
                }
            }

            return Task.CompletedTask;
        };

        var options = new MqttClientOptionsBuilder()
            .WithTcpServer(host, port)
            .WithClientId("jarvis-bootstrap-" + Guid.NewGuid().ToString("N")[..8])
            .WithCredentials(adminUsername, adminPassword)
            .WithProtocolVersion(MqttProtocolVersion.V500)
            .WithCleanStart(true)
            .Build();
        var connect = await client.ConnectAsync(options, cancellationToken);
        if (connect.ResultCode != MqttClientConnectResultCode.Success)
        {
            throw new InvalidOperationException($"Admin connection rejected: {connect.ResultCode}");
        }

        await client.SubscribeAsync(
            new MqttClientSubscribeOptionsBuilder().WithTopicFilter(MqttCloudConnection.DynsecResponseTopic, MqttQualityOfServiceLevel.AtLeastOnce).Build(),
            cancellationToken);

        async Task<JsonObject> Run(JsonObject command, bool tolerateExisting = true)
        {
            var correlation = Guid.NewGuid().ToString("N");
            command["correlationData"] = correlation;
            var tcs = new TaskCompletionSource<JsonObject>(TaskCreationOptions.RunContinuationsAsynchronously);
            responses[correlation] = tcs;
            var body = JsonSerializer.SerializeToUtf8Bytes(new JsonObject { ["commands"] = new JsonArray(command) });
            await client.PublishAsync(
                new MqttApplicationMessageBuilder().WithTopic(MqttCloudConnection.DynsecCommandTopic).WithPayload(body)
                    .WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce).Build(),
                cancellationToken);
            var response = await tcs.Task.WaitAsync(TimeSpan.FromSeconds(10), cancellationToken);
            var error = response["error"]?.GetValue<string>();
            if (!string.IsNullOrEmpty(error) && !(tolerateExisting && error.Contains("already exists", StringComparison.OrdinalIgnoreCase)))
            {
                throw new BrokerCommandException(command["command"]!.GetValue<string>(), error);
            }

            return response;
        }

        await Run(DynsecCommands.DefaultDenyAcl());
        await Run(DynsecCommands.CreateCloudRole());
        foreach (var acl in DynsecCommands.CloudAcls().OfType<JsonObject>())
        {
            var add = (JsonObject)acl.DeepClone();
            add["command"] = "addRoleACL";
            add["rolename"] = DynsecCommands.CloudRole;
            await Run(add);
        }

        var created = await Run(DynsecCommands.CreateClient("jarvis-cloud", cloudPassword, null, DynsecCommands.CloudRole));
        if (created["error"] is not null)
        {
            await Run(new JsonObject { ["command"] = "setClientPassword", ["username"] = "jarvis-cloud", ["password"] = cloudPassword }, false);
            try
            {
                await Run(new JsonObject { ["command"] = "addClientRole", ["username"] = "jarvis-cloud", ["rolename"] = DynsecCommands.CloudRole });
            }
            catch (BrokerCommandException)
            {
                // El client ya tenía el rol.
            }
        }

        logger.LogInformation("Dynamic Security bootstrap completed (roles, default deny, jarvis-cloud client)");
        if (deleteAdmin)
        {
            try
            {
                await Run(DynsecCommands.Simple("deleteClient", "username", adminUsername), false);
            }
            catch (MQTTnet.Exceptions.MqttClientUnexpectedDisconnectReceivedException)
            {
                // Borrar el client con el que estamos conectados hace que el broker cierre esta sesión: es el éxito esperado.
            }

            logger.LogInformation("Temporary admin client {Admin} deleted", adminUsername);
            return;
        }

        await client.DisconnectAsync(cancellationToken: cancellationToken);
    }
}
