using System.Text.Json;
using System.Text.Json.Serialization;

namespace Jarvis.Contracts.Mqtt;

/// <summary>
/// Envelope común de todo mensaje MQTT (docs/03-protocols/mqtt-protocol.md §3).
/// Los campos son anulables para poder detectar mensajes incompletos al validar.
/// </summary>
public sealed record MqttEnvelope
{
    public const int CurrentVersion = 1;
    public const int MaxMessageBytes = 64 * 1024;

    [JsonPropertyName("v")]
    public int? V { get; init; }

    public Guid? Id { get; init; }

    public string? Type { get; init; }

    public DateTimeOffset? Ts { get; init; }

    public Guid? DeviceId { get; init; }

    public Guid? CorrelationId { get; init; }

    public Guid? CausationId { get; init; }

    public JsonElement Data { get; init; }

    public static MqttEnvelope Create<T>(
        string type,
        Guid deviceId,
        T data,
        DateTimeOffset ts,
        Guid? correlationId = null,
        Guid? causationId = null,
        Guid? id = null) =>
        new()
        {
            V = CurrentVersion,
            Id = id ?? Guid.CreateVersion7(ts),
            Type = type,
            Ts = ts,
            DeviceId = deviceId,
            CorrelationId = correlationId,
            CausationId = causationId,
            Data = JarvisJson.ToElement(data),
        };

    public byte[] ToBytes() => JsonSerializer.SerializeToUtf8Bytes(this, JarvisJson.Options);

    public static MqttEnvelope Parse(ReadOnlySpan<byte> utf8) =>
        JsonSerializer.Deserialize<MqttEnvelope>(utf8, JarvisJson.Options)
        ?? throw new JsonException("Empty envelope.");

    public T ReadData<T>() =>
        Data.ValueKind == JsonValueKind.Object
            ? Data.Deserialize<T>(JarvisJson.Options) ?? throw new JsonException("Empty data.")
            : throw new JsonException("data must be an object.");
}
