using System.Buffers;
using System.Threading.Channels;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Mqtt;
using Jarvis.Contracts.Mqtt;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using MQTTnet;
using MQTTnet.Formatter;
using MQTTnet.Protocol;

namespace Jarvis.Infrastructure.Mqtt;

/// <summary>
/// HU-021: conexión MQTT del Cloud. Sesión persistente (clean start false), suscripción a <c>up/#</c>,
/// PUBACK manual después de procesar (commit en PostgreSQL), reconexión con backoff y orden por device.
/// </summary>
public sealed class MqttCloudConnection : BackgroundService, IMqttConnectionState
{
    public const string DynsecResponseTopic = "$CONTROL/dynamic-security/v1/response";
    public const string DynsecCommandTopic = "$CONTROL/dynamic-security/v1";

    private readonly MqttOptions _options;
    private readonly MqttInboundProcessor _processor;
    private readonly JarvisMetrics _metrics;
    private readonly ILogger<MqttCloudConnection> _logger;
    private readonly IMqttClient _client;
    private readonly Channel<MqttApplicationMessageReceivedEventArgs>[] _partitions;
    private readonly SemaphoreSlim _connectLock = new(1, 1);

    public MqttCloudConnection(
        IOptions<MqttOptions> options,
        MqttInboundProcessor processor,
        JarvisMetrics metrics,
        ILogger<MqttCloudConnection> logger)
    {
        _options = options.Value;
        _processor = processor;
        _metrics = metrics;
        _logger = logger;
        _client = new MqttClientFactory().CreateMqttClient();
        _client.ApplicationMessageReceivedAsync += OnMessageReceivedAsync;
        _client.DisconnectedAsync += e =>
        {
            _metrics.SetMqttConnected(false);
            if (e.ClientWasConnected)
            {
                _logger.LogWarning("MQTT cloud client disconnected: {Reason}", e.Reason);
            }

            return Task.CompletedTask;
        };
        _partitions = Enumerable.Range(0, Math.Max(1, _options.MaxConcurrentMessages))
            .Select(_ => Channel.CreateUnbounded<MqttApplicationMessageReceivedEventArgs>(new UnboundedChannelOptions { SingleReader = true }))
            .ToArray();
    }

    public bool IsConnected => _client.IsConnected;

    /// <summary>Recibe respuestas de Dynamic Security (<c>$CONTROL/dynamic-security/v1/response</c>).</summary>
    public event Action<ReadOnlyMemory<byte>>? DynsecResponseReceived;

    public async Task<bool> PublishAsync(string topic, byte[] payload, CancellationToken cancellationToken)
    {
        if (!_client.IsConnected)
        {
            return false;
        }

        var message = new MqttApplicationMessageBuilder()
            .WithTopic(topic)
            .WithPayload(payload)
            .WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce)
            .WithRetainFlag(false)
            .Build();
        try
        {
            var result = await _client.PublishAsync(message, cancellationToken);
            return result.IsSuccess;
        }
        catch (Exception exception) when (exception is not OperationCanceledException)
        {
            _logger.LogWarning(exception, "MQTT publish to {Topic} failed", topic);
            return false;
        }
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var workers = _partitions.Select(p => Task.Run(() => ProcessPartitionAsync(p.Reader, stoppingToken), stoppingToken)).ToArray();
        var delay = TimeSpan.FromSeconds(1);
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                if (!_client.IsConnected)
                {
                    await ConnectAsync(stoppingToken);
                    delay = TimeSpan.FromSeconds(1);
                }
                else if (!await _client.TryPingAsync(stoppingToken))
                {
                    _logger.LogWarning("MQTT ping failed; reconnecting");
                    await _client.DisconnectAsync(new MqttClientDisconnectOptions(), CancellationToken.None);
                }

                await Task.Delay(TimeSpan.FromSeconds(5), stoppingToken);
            }
            catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
            {
                break;
            }
            catch (Exception exception)
            {
                _logger.LogWarning("MQTT connection to {Host}:{Port} failed: {Message}", _options.Host, _options.Port, exception.Message);
                await Task.Delay(delay, stoppingToken).ContinueWith(_ => { }, TaskScheduler.Default);
                delay = TimeSpan.FromSeconds(Math.Min(30, delay.TotalSeconds * 2));
            }
        }

        foreach (var partition in _partitions)
        {
            partition.Writer.TryComplete();
        }

        if (_client.IsConnected)
        {
            await _client.DisconnectAsync(new MqttClientDisconnectOptions { SessionExpiryInterval = (uint)_options.SessionExpirySeconds }, CancellationToken.None);
        }

        await Task.WhenAll(workers).ContinueWith(_ => { }, TaskScheduler.Default);
    }

    public override void Dispose()
    {
        _client.Dispose();
        _connectLock.Dispose();
        base.Dispose();
    }

    private async Task ConnectAsync(CancellationToken cancellationToken)
    {
        await _connectLock.WaitAsync(cancellationToken);
        try
        {
            if (_client.IsConnected)
            {
                return;
            }

            var builder = new MqttClientOptionsBuilder()
                .WithTcpServer(_options.Host, _options.Port)
                .WithClientId($"jarvis-cloud-{_options.InstanceId}")
                .WithCredentials(_options.Username, _options.Password)
                .WithProtocolVersion(MqttProtocolVersion.V500)
                .WithCleanStart(false)
                .WithSessionExpiryInterval((uint)_options.SessionExpirySeconds)
                .WithKeepAlivePeriod(TimeSpan.FromSeconds(30))
                .WithTimeout(TimeSpan.FromSeconds(10));
            if (_options.UseTls)
            {
                builder = builder.WithTlsOptions(tls => tls.UseTls().WithTargetHost(_options.Host));
            }

            var result = await _client.ConnectAsync(builder.Build(), cancellationToken);
            if (result.ResultCode != MqttClientConnectResultCode.Success)
            {
                throw new InvalidOperationException($"CONNACK {result.ResultCode}");
            }

            var subscribe = new MqttClientSubscribeOptionsBuilder()
                .WithTopicFilter(new MqttTopicFilterBuilder().WithTopic(MqttTopics.CloudSubscription).WithAtLeastOnceQoS().Build())
                .WithTopicFilter(new MqttTopicFilterBuilder().WithTopic(DynsecResponseTopic).WithAtLeastOnceQoS().Build())
                .Build();
            await _client.SubscribeAsync(subscribe, cancellationToken);
            _metrics.SetMqttConnected(true);
            _logger.LogInformation(
                "MQTT cloud client connected to {Host}:{Port} (session present: {SessionPresent})",
                _options.Host,
                _options.Port,
                result.IsSessionPresent);
        }
        finally
        {
            _connectLock.Release();
        }
    }

    private Task OnMessageReceivedAsync(MqttApplicationMessageReceivedEventArgs e)
    {
        var topic = e.ApplicationMessage.Topic;
        if (topic == DynsecResponseTopic)
        {
            DynsecResponseReceived?.Invoke(e.ApplicationMessage.Payload.ToArray());
            return Task.CompletedTask;
        }

        // Diferir el PUBACK hasta procesar; mensajes del mismo device van a la misma partición (orden preservado).
        e.AutoAcknowledge = false;
        var partition = (int)((uint)StringComparer.Ordinal.GetHashCode(PartitionKey(topic)) % (uint)_partitions.Length);
        _partitions[partition].Writer.TryWrite(e);
        return Task.CompletedTask;
    }

    private static string PartitionKey(string topic) =>
        MqttTopics.TryParse(topic, out var deviceId, out _, out _) ? deviceId.ToString("N") : topic;

    private async Task ProcessPartitionAsync(ChannelReader<MqttApplicationMessageReceivedEventArgs> reader, CancellationToken stoppingToken)
    {
        await foreach (var e in reader.ReadAllAsync(CancellationToken.None))
        {
            var payload = e.ApplicationMessage.Payload.ToArray();
            for (var attempt = 1; ; attempt++)
            {
                try
                {
                    await _processor.ProcessAsync(e.ApplicationMessage.Topic, payload, stoppingToken);
                    break;
                }
                catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
                {
                    // Sin PUBACK: el broker reentregará el mensaje al reconectar (sesión persistente).
                    return;
                }
                catch (Exception exception) when (attempt < 3)
                {
                    _logger.LogWarning(exception, "Processing MQTT message on {Topic} failed (attempt {Attempt}); retrying", e.ApplicationMessage.Topic, attempt);
                    await Task.Delay(TimeSpan.FromMilliseconds(200 * attempt), CancellationToken.None);
                }
                catch (Exception exception)
                {
                    // Se confirma para no bloquear la cola con un mensaje venenoso.
                    _logger.LogError(exception, "Processing MQTT message on {Topic} failed permanently; acknowledging", e.ApplicationMessage.Topic);
                    break;
                }
            }

            try
            {
                await e.AcknowledgeAsync(CancellationToken.None);
            }
            catch (Exception exception)
            {
                _logger.LogDebug(exception, "PUBACK could not be sent (connection lost); the broker will redeliver");
            }
        }
    }
}

/// <summary>HU-023: construye envelopes y publica QoS 1 sin retain en el topic down del tipo de mensaje.</summary>
public sealed class MqttDeviceMessagePublisher(
    MqttCloudConnection connection,
    JarvisMetrics metrics,
    TimeProvider clock,
    ILogger<MqttDeviceMessagePublisher> logger) : IDeviceMessagePublisher
{
    public async Task<bool> PublishAsync<T>(
        Guid deviceId,
        string messageType,
        T data,
        Guid? correlationId,
        Guid? causationId,
        CancellationToken cancellationToken)
    {
        var envelope = MqttEnvelope.Create(messageType, deviceId, data, clock.GetUtcNow(), correlationId, causationId);
        var topic = MqttTopics.Down(deviceId, MqttTopics.ChannelFor(messageType));
        var published = await connection.PublishAsync(topic, envelope.ToBytes(), cancellationToken);
        if (published)
        {
            metrics.MqttPublished(messageType);
        }
        else
        {
            logger.LogWarning("Could not publish {MessageType} to {DeviceId}", messageType, deviceId);
        }

        return published;
    }
}
