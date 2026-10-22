using System.Buffers.Text;
using System.Collections.Concurrent;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Threading.Channels;
using Jarvis.Contracts;
using Jarvis.Domain.Events;

namespace Jarvis.Application.Common;

/// <summary>Recibe los eventos ya confirmados en PostgreSQL (llamado por el DbContext tras el commit).</summary>
public interface IPostCommitEventSink
{
    void Publish(IReadOnlyList<EventRecord> events);
}

/// <summary>Consumidor de eventos confirmados (realtime, notificaciones al device origen, señales).</summary>
public interface IEventSubscriber
{
    Task HandleAsync(EventRecord record, CancellationToken cancellationToken);
}

/// <summary>Cola en memoria de eventos confirmados, consumida en orden por un único lector.</summary>
public sealed class PostCommitEventChannel : IPostCommitEventSink
{
    private readonly Channel<EventRecord> _channel = Channel.CreateUnbounded<EventRecord>(
        new UnboundedChannelOptions { SingleReader = true });

    public ChannelReader<EventRecord> Reader => _channel.Reader;

    public void Publish(IReadOnlyList<EventRecord> events)
    {
        foreach (var record in events)
        {
            _channel.Writer.TryWrite(record);
        }
    }
}

/// <summary>Señal para despertar al dispatcher (creación de tarea, hello de un device, timeouts).</summary>
public sealed class DispatchSignal : IDisposable
{
    private readonly SemaphoreSlim _signal = new(0, 1);

    public void Wake()
    {
        try
        {
            _signal.Release();
        }
        catch (SemaphoreFullException)
        {
            // Ya estaba señalizado.
        }
    }

    public Task<bool> WaitAsync(TimeSpan timeout, CancellationToken cancellationToken) => _signal.WaitAsync(timeout, cancellationToken);

    public void Dispose() => _signal.Dispose();
}

/// <summary>Último ciclo exitoso de cada background service, usado por readiness.</summary>
public sealed class ServiceHeartbeats(TimeProvider clock)
{
    private readonly ConcurrentDictionary<string, (DateTimeOffset At, TimeSpan Interval)> _beats = new();

    public void Beat(string service, TimeSpan expectedInterval) => _beats[service] = (clock.GetUtcNow(), expectedInterval);

    public IReadOnlyDictionary<string, (DateTimeOffset At, TimeSpan Interval)> Snapshot() => _beats;

    /// <summary>Servicios cuyo último latido es más viejo que 3× su intervalo.</summary>
    public IReadOnlyList<string> Stalled()
    {
        var now = clock.GetUtcNow();
        return _beats.Where(b => now - b.Value.At > b.Value.Interval * 3).Select(b => b.Key).ToList();
    }
}

public static class SecretGenerator
{
    public const string DeviceSecretPrefix = "jdv_";
    public const string RefreshTokenPrefix = "jrt_";

    public static string NewDeviceSecret() => DeviceSecretPrefix + Base64Url.EncodeToString(RandomNumberGenerator.GetBytes(32));

    public static string NewRefreshToken() => RefreshTokenPrefix + Base64Url.EncodeToString(RandomNumberGenerator.GetBytes(32));

    public static byte[] Sha256(string value) => SHA256.HashData(Encoding.UTF8.GetBytes(value));

    public static string Hint(string secret) => secret[^4..];
}

/// <summary>Cursor opaco de paginación keyset (<c>created_at</c>, <c>id</c>).</summary>
public static class KeysetCursor
{
    public static string Encode(DateTimeOffset createdAt, Guid id) =>
        Base64Url.EncodeToString(Encoding.UTF8.GetBytes($"{createdAt.UtcTicks.ToString(CultureInfo.InvariantCulture)}|{id:N}"));

    public static bool TryDecode(string? cursor, out DateTimeOffset createdAt, out Guid id)
    {
        createdAt = default;
        id = default;
        if (string.IsNullOrEmpty(cursor))
        {
            return false;
        }

        try
        {
            var parts = Encoding.UTF8.GetString(Base64Url.DecodeFromChars(cursor)).Split('|');
            if (parts.Length == 2
                && long.TryParse(parts[0], NumberStyles.None, CultureInfo.InvariantCulture, out var ticks)
                && Guid.TryParseExact(parts[1], "N", out id))
            {
                createdAt = new DateTimeOffset(ticks, TimeSpan.Zero);
                return true;
            }
        }
        catch (FormatException)
        {
        }

        return false;
    }
}

public static class JsonText
{
    public static JsonElement ToElement(string? json) =>
        string.IsNullOrEmpty(json) ? JarvisJson.EmptyObject : JarvisJson.ParseElement(json);

    public static JsonElement? ToNullableElement(string? json) => json is null ? null : JarvisJson.ParseElement(json);

    public static string Serialize(object? value) => JsonSerializer.Serialize(value, JarvisJson.Options);

    public static string Compact(JsonElement element) => JsonSerializer.Serialize(element);
}

public static class Limits
{
    public static int ClampLimit(int? limit, int defaultValue, int max) => limit is null or < 1 ? defaultValue : Math.Min(limit.Value, max);
}
