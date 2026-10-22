using System.Buffers;
using System.Net.Security;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Jarvis.Contracts.Mqtt;
using MQTTnet;
using MQTTnet.Formatter;
using MQTTnet.Protocol;

namespace Jarvis.Simulators;

public sealed record DeviceSettings
{
    public required Guid DeviceId { get; init; }

    public required string Secret { get; init; }

    public string Host { get; init; } = "localhost";

    public int Port { get; init; } = 8883;

    public bool UseTls { get; init; } = true;

    /// <summary>CA raíz adicional (PEM) para brokers con certificado propio (desarrollo/tests).</summary>
    public string? CaFile { get; init; }

    public int KeepAliveSeconds { get; init; } = 60;

    public TimeSpan AuthFailureBackoff { get; init; } = TimeSpan.FromMinutes(5);

    public string SoftwareVersion { get; init; } = "jarvis-simulator/0.1.0";

    public string Platform { get; init; } = "simulator";

    public IReadOnlyList<string> Capabilities { get; init; } = [];

    public IReadOnlyList<string> SupportedTaskTypes { get; init; } = [];
}

/// <summary>
/// Conexión MQTT de un device según docs/03-protocols/mqtt-protocol.md §5: clean start, LWT con sessionId,
/// suscripción a <c>down/#</c>, <c>presence.hello</c> y heartbeats, reconexión con backoff exponencial y jitter.
/// </summary>
public sealed class DeviceConnection : IAsyncDisposable
{
    private readonly DeviceSettings _settings;
    private readonly Func<IReadOnlyList<Guid>> _openTaskIds;
    private readonly Func<bool> _busy;
    private readonly Action<string> _log;
    private readonly SemaphoreSlim _publishLock = new(1, 1);
    private IMqttClient? _client;
    private CancellationTokenSource? _lifetime;
    private Task? _loop;
    private TaskCompletionSource _helloAck = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private volatile bool _ready;

    public DeviceConnection(DeviceSettings settings, Func<IReadOnlyList<Guid>> openTaskIds, Func<bool> busy, Action<string> log)
    {
        _settings = settings;
        _openTaskIds = openTaskIds;
        _busy = busy;
        _log = log;
    }

    public event Func<MqttEnvelope, Task>? MessageReceived;

    /// <summary>Se dispara tras recibir <c>presence.hello.ack</c> (conexión lista).</summary>
    public event Func<Task>? Ready;

    public string SessionId { get; private set; } = string.Empty;

    public bool IsConnected => _client?.IsConnected == true;

    public bool PauseHeartbeats { get; set; }

    public int HeartbeatIntervalSeconds { get; private set; } = 30;

    public Guid DeviceId => _settings.DeviceId;

    /// <summary>True entre <c>presence.hello.ack</c> y la pérdida de la conexión.</summary>
    public bool IsReady => _ready && IsConnected;

    public async Task WaitReadyAsync(TimeSpan timeout)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (!IsReady)
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException($"Device {_settings.DeviceId} not ready within {timeout}.");
            }

            await Task.Delay(50);
        }
    }

    public void Start()
    {
        _lifetime = new CancellationTokenSource();
        _loop = Task.Run(() => RunAsync(_lifetime.Token));
    }

    public async Task PublishAsync<T>(string channel, string type, T data, Guid? correlationId = null, Guid? causationId = null, MqttEnvelope? envelope = null)
    {
        envelope ??= MqttEnvelope.Create(type, _settings.DeviceId, data, DateTimeOffset.UtcNow, correlationId, causationId);
        await PublishEnvelopeAsync(channel, envelope);
    }

    public async Task PublishEnvelopeAsync(string channel, MqttEnvelope envelope, MqttQualityOfServiceLevel qos = MqttQualityOfServiceLevel.AtLeastOnce)
    {
        var client = _client;
        if (client is not { IsConnected: true })
        {
            return;
        }

        await _publishLock.WaitAsync();
        try
        {
            await client.PublishAsync(new MqttApplicationMessageBuilder()
                .WithTopic(MqttTopics.Up(_settings.DeviceId, channel))
                .WithPayload(envelope.ToBytes())
                .WithQualityOfServiceLevel(qos)
                .WithRetainFlag(false)
                .Build());
        }
        catch (Exception exception) when (exception is not OperationCanceledException)
        {
            _log($"publish failed: {exception.Message}");
        }
        finally
        {
            _publishLock.Release();
        }
    }

    /// <summary>Publica en un topic arbitrario (solo para pruebas de ACL).</summary>
    public async Task<bool> TryPublishRawAsync(string topic, byte[] payload)
    {
        if (_client is not { IsConnected: true } client)
        {
            return false;
        }

        var result = await client.PublishAsync(new MqttApplicationMessageBuilder().WithTopic(topic).WithPayload(payload)
            .WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce).Build());
        return result.IsSuccess;
    }

    /// <summary>Intenta suscribirse a un filtro arbitrario (solo para pruebas de ACL). True si el broker lo concedió.</summary>
    public async Task<bool> TrySubscribeRawAsync(string filter)
    {
        if (_client is not { IsConnected: true } client)
        {
            return false;
        }

        var result = await client.SubscribeAsync(new MqttClientSubscribeOptionsBuilder().WithTopicFilter(filter, MqttQualityOfServiceLevel.AtLeastOnce).Build());
        return result.Items.All(i => (int)i.ResultCode < 0x80);
    }

    /// <summary>Desconexión que hace publicar el LWT (simula caída abrupta) sin reconectar.</summary>
    public async Task CrashAsync()
    {
        _lifetime?.Cancel();
        if (_client is { IsConnected: true } client)
        {
            await client.DisconnectAsync(new MqttClientDisconnectOptions { Reason = MqttClientDisconnectOptionsReason.DisconnectWithWillMessage });
        }
    }

    /// <summary>Corta la conexión (con LWT) y deja que el loop reconecte con una nueva sesión.</summary>
    public async Task DropConnectionAsync()
    {
        if (_client is { IsConnected: true } client)
        {
            await client.DisconnectAsync(new MqttClientDisconnectOptions { Reason = MqttClientDisconnectOptionsReason.DisconnectWithWillMessage });
        }
    }

    public async Task StopAsync(string reason = "shutdown")
    {
        if (_client is { IsConnected: true })
        {
            await PublishAsync(TopicChannels.Presence, MessageTypes.PresenceGoodbye, new PresenceGoodbyeData(SessionId, reason));
            await _client.DisconnectAsync(new MqttClientDisconnectOptions { Reason = MqttClientDisconnectOptionsReason.NormalDisconnection });
        }

        _lifetime?.Cancel();
        if (_loop is not null)
        {
            await _loop.ContinueWith(_ => { }, TaskScheduler.Default);
        }
    }

    public async ValueTask DisposeAsync()
    {
        _lifetime?.Cancel();
        if (_loop is not null)
        {
            await _loop.ContinueWith(_ => { }, TaskScheduler.Default);
        }

        _client?.Dispose();
        _lifetime?.Dispose();
        _publishLock.Dispose();
    }

    private async Task RunAsync(CancellationToken cancellationToken)
    {
        var delay = TimeSpan.FromSeconds(1);
        while (!cancellationToken.IsCancellationRequested)
        {
            try
            {
                await ConnectAsync(cancellationToken);
                delay = TimeSpan.FromSeconds(1);
                var lastHeartbeat = DateTimeOffset.UtcNow;
                while (!cancellationToken.IsCancellationRequested && _client!.IsConnected)
                {
                    await Task.Delay(250, cancellationToken);
                    if (!PauseHeartbeats && DateTimeOffset.UtcNow - lastHeartbeat >= TimeSpan.FromSeconds(HeartbeatIntervalSeconds))
                    {
                        lastHeartbeat = DateTimeOffset.UtcNow;
                        var heartbeat = MqttEnvelope.Create(
                            MessageTypes.PresenceHeartbeat,
                            _settings.DeviceId,
                            new PresenceHeartbeatData(SessionId, _busy(), _openTaskIds().Count),
                            DateTimeOffset.UtcNow);
                        await PublishEnvelopeAsync(TopicChannels.Presence, heartbeat, MqttQualityOfServiceLevel.AtMostOnce);
                    }
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return;
            }
            catch (AuthenticationRejectedException exception)
            {
                _log($"authentication rejected ({exception.Message}); credential revoked or rotated? backing off {_settings.AuthFailureBackoff}");
                await Task.Delay(_settings.AuthFailureBackoff, cancellationToken).ContinueWith(_ => { }, TaskScheduler.Default);
                continue;
            }
            catch (Exception exception)
            {
                _log($"connection error: {exception.Message}");
            }

            // Backoff exponencial con ±20 % de jitter (1 s → 60 s).
            var jitter = 0.8 + (Random.Shared.NextDouble() * 0.4);
            await Task.Delay(delay * jitter, cancellationToken).ContinueWith(_ => { }, TaskScheduler.Default);
            delay = TimeSpan.FromSeconds(Math.Min(60, delay.TotalSeconds * 2));
        }
    }

    private async Task ConnectAsync(CancellationToken cancellationToken)
    {
        _ready = false;
        _client?.Dispose();
        _helloAck = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        SessionId = "s-" + Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(8));
        var will = MqttEnvelope.Create(MessageTypes.PresenceOffline, _settings.DeviceId, new PresenceOfflineData(SessionId), DateTimeOffset.UtcNow);
        var id = _settings.DeviceId.ToString("D");

        var builder = new MqttClientOptionsBuilder()
            .WithTcpServer(_settings.Host, _settings.Port)
            .WithClientId(id)
            .WithCredentials(id, _settings.Secret)
            .WithProtocolVersion(MqttProtocolVersion.V500)
            .WithCleanStart(true)
            .WithKeepAlivePeriod(TimeSpan.FromSeconds(_settings.KeepAliveSeconds))
            .WithTimeout(TimeSpan.FromSeconds(10))
            .WithWillTopic(MqttTopics.Up(_settings.DeviceId, TopicChannels.Presence))
            .WithWillPayload(will.ToBytes())
            .WithWillQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce)
            .WithWillRetain(false);
        if (_settings.UseTls)
        {
            builder = builder.WithTlsOptions(tls =>
            {
                tls.UseTls().WithTargetHost(_settings.Host);
                if (_settings.CaFile is not null)
                {
                    var ca = X509Certificate2.CreateFromPem(File.ReadAllText(_settings.CaFile));
                    tls.WithCertificateValidationHandler(args => ValidateWithCustomRoot(args.Certificate, args.SslPolicyErrors, ca));
                }
            });
        }

        var client = new MqttClientFactory().CreateMqttClient();
        client.ApplicationMessageReceivedAsync += OnMessageAsync;
        _client = client;
        var result = await client.ConnectAsync(builder.Build(), cancellationToken);
        if (result.ResultCode is MqttClientConnectResultCode.BadUserNameOrPassword or MqttClientConnectResultCode.NotAuthorized)
        {
            throw new AuthenticationRejectedException(result.ResultCode.ToString());
        }

        if (result.ResultCode != MqttClientConnectResultCode.Success)
        {
            throw new InvalidOperationException($"CONNACK {result.ResultCode}");
        }

        await client.SubscribeAsync(
            new MqttClientSubscribeOptionsBuilder().WithTopicFilter(MqttTopics.DeviceSubscription(_settings.DeviceId), MqttQualityOfServiceLevel.AtLeastOnce).Build(),
            cancellationToken);

        await PublishAsync(
            TopicChannels.Presence,
            MessageTypes.PresenceHello,
            new PresenceHelloData(
                SessionId,
                "1.0",
                _settings.SoftwareVersion,
                _settings.Platform,
                _settings.Capabilities,
                _settings.SupportedTaskTypes,
                _openTaskIds(),
                _busy()));

        await _helloAck.Task.WaitAsync(TimeSpan.FromSeconds(10), cancellationToken);
        _ready = true;
        _log($"connected (session {SessionId})");
        if (Ready is not null)
        {
            await Ready.Invoke();
        }
    }

    private async Task OnMessageAsync(MqttApplicationMessageReceivedEventArgs e)
    {
        MqttEnvelope envelope;
        try
        {
            envelope = MqttEnvelope.Parse(e.ApplicationMessage.Payload.ToArray());
        }
        catch (System.Text.Json.JsonException)
        {
            _log("invalid message ignored");
            return;
        }

        if (envelope.Type == MessageTypes.PresenceHelloAck)
        {
            var ack = envelope.ReadData<PresenceHelloAckData>();
            if (ack.SessionId == SessionId)
            {
                HeartbeatIntervalSeconds = Math.Max(1, ack.HeartbeatIntervalSeconds);
                _helloAck.TrySetResult();
            }

            return;
        }

        if (MessageReceived is not null)
        {
            // Fuera del hilo del cliente MQTT para poder publicar respuestas sin bloquear la recepción.
            _ = Task.Run(async () =>
            {
                try
                {
                    await MessageReceived.Invoke(envelope);
                }
                catch (Exception exception)
                {
                    _log($"handler error: {exception.Message}");
                }
            });
        }
    }

    private static bool ValidateWithCustomRoot(X509Certificate? certificate, SslPolicyErrors errors, X509Certificate2 root)
    {
        if (errors == SslPolicyErrors.None)
        {
            return true;
        }

        if (certificate is null || errors.HasFlag(SslPolicyErrors.RemoteCertificateNameMismatch) || errors.HasFlag(SslPolicyErrors.RemoteCertificateNotAvailable))
        {
            return false;
        }

        using var leaf = X509CertificateLoader.LoadCertificate(certificate.GetRawCertData());
        using var chain = new X509Chain();
        chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
        chain.ChainPolicy.CustomTrustStore.Add(root);
        chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
        return chain.Build(leaf);
    }
}

public sealed class AuthenticationRejectedException(string message) : Exception(message);
