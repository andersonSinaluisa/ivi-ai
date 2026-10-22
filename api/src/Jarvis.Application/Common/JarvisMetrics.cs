using System.Collections.Concurrent;
using System.Diagnostics.Metrics;

namespace Jarvis.Application.Common;

/// <summary>Métricas del meter <c>Jarvis.Cloud</c> (docs/06-infrastructure/observability.md). Nunca usa ids como tags.</summary>
public sealed class JarvisMetrics : IDisposable
{
    public const string MeterName = "Jarvis.Cloud";

    private readonly Meter _meter;
    private readonly Counter<long> _tasksCreated;
    private readonly Counter<long> _tasksTerminal;
    private readonly Counter<long> _dispatchAttempts;
    private readonly Histogram<double> _deliveryLatency;
    private readonly Histogram<double> _executionDuration;
    private readonly Counter<long> _mqttReceived;
    private readonly Counter<long> _mqttPublished;
    private readonly Counter<long> _securityViolations;
    private readonly Counter<long> _authFailures;
    private readonly Counter<long> _rateLimited;
    private readonly ConcurrentDictionary<string, long> _devicesByPresence = new();
    private long _pendingTasks;
    private int _mqttConnected;

    public JarvisMetrics(IMeterFactory meterFactory)
    {
        _meter = meterFactory.Create(MeterName);
        _tasksCreated = _meter.CreateCounter<long>("jarvis_tasks_created_total");
        _tasksTerminal = _meter.CreateCounter<long>("jarvis_tasks_terminal_total");
        _dispatchAttempts = _meter.CreateCounter<long>("jarvis_task_dispatch_attempts_total");
        _deliveryLatency = _meter.CreateHistogram<double>("jarvis_task_delivery_latency_seconds", "s");
        _executionDuration = _meter.CreateHistogram<double>("jarvis_task_execution_duration_seconds", "s");
        _mqttReceived = _meter.CreateCounter<long>("jarvis_mqtt_messages_received_total");
        _mqttPublished = _meter.CreateCounter<long>("jarvis_mqtt_messages_published_total");
        _securityViolations = _meter.CreateCounter<long>("jarvis_security_violations_total");
        _authFailures = _meter.CreateCounter<long>("jarvis_auth_failures_total");
        _rateLimited = _meter.CreateCounter<long>("jarvis_rate_limited_total");
        _meter.CreateObservableGauge("jarvis_tasks_pending", () => Interlocked.Read(ref _pendingTasks));
        _meter.CreateObservableGauge(
            "jarvis_devices_by_presence",
            () => _devicesByPresence.Select(p => new Measurement<long>(p.Value, new KeyValuePair<string, object?>("presence_state", p.Key))));
        _meter.CreateObservableGauge("jarvis_mqtt_connected", () => Volatile.Read(ref _mqttConnected));
    }

    public void TaskCreated(string channel, string requestedByType) =>
        _tasksCreated.Add(1, new("channel", channel), new("requested_by_type", requestedByType));

    public void TaskTerminal(string status, string? errorCode) =>
        _tasksTerminal.Add(1, new("status", status), new("error_code", errorCode ?? "none"));

    public void DispatchAttempt(int attempt) => _dispatchAttempts.Add(1, new KeyValuePair<string, object?>("attempt", Math.Min(attempt, 10)));

    public void DeliveryLatency(TimeSpan latency) => _deliveryLatency.Record(latency.TotalSeconds);

    public void ExecutionDuration(TimeSpan duration, string status) => _executionDuration.Record(duration.TotalSeconds, new KeyValuePair<string, object?>("status", status));

    public void MqttReceived(string messageType, string result) =>
        _mqttReceived.Add(1, new("message_type", messageType), new("result", result));

    public void MqttPublished(string messageType) => _mqttPublished.Add(1, new KeyValuePair<string, object?>("message_type", messageType));

    public void SecurityViolation(string rule) => _securityViolations.Add(1, new KeyValuePair<string, object?>("rule", rule));

    public void AuthFailure(string kind) => _authFailures.Add(1, new KeyValuePair<string, object?>("kind", kind));

    public void RateLimited(string policy) => _rateLimited.Add(1, new KeyValuePair<string, object?>("policy", policy));

    public void SetPendingTasks(long count) => Interlocked.Exchange(ref _pendingTasks, count);

    public void SetDevicesByPresence(IReadOnlyDictionary<string, long> counts)
    {
        foreach (var key in new[] { "Online", "Stale", "Offline" })
        {
            _devicesByPresence[key] = counts.TryGetValue(key, out var value) ? value : 0;
        }
    }

    public void SetMqttConnected(bool connected) => Volatile.Write(ref _mqttConnected, connected ? 1 : 0);

    public void Dispose() => _meter.Dispose();
}
