using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Devices;
using Jarvis.Infrastructure.Persistence;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Diagnostics.HealthChecks;

namespace Jarvis.Api.Health;

/// <summary>PostgreSQL alcanzable y sin migraciones pendientes → si no, Unhealthy (docs/06-infrastructure/observability.md).</summary>
public sealed class PostgresHealthCheck(IServiceScopeFactory scopes) : IHealthCheck
{
    private static DateTimeOffset _migrationsCheckedAt = DateTimeOffset.MinValue;
    private static bool _migrationsApplied;

    public async Task<HealthCheckResult> CheckHealthAsync(HealthCheckContext context, CancellationToken cancellationToken = default)
    {
        try
        {
            await using var scope = scopes.CreateAsyncScope();
            var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(TimeSpan.FromSeconds(2));
            await db.Database.ExecuteSqlRawAsync("SELECT 1", timeout.Token);

            if (!_migrationsApplied || DateTimeOffset.UtcNow - _migrationsCheckedAt > TimeSpan.FromMinutes(1))
            {
                _migrationsApplied = !(await db.Database.GetPendingMigrationsAsync(timeout.Token)).Any();
                _migrationsCheckedAt = DateTimeOffset.UtcNow;
            }

            return _migrationsApplied
                ? HealthCheckResult.Healthy()
                : HealthCheckResult.Unhealthy("Pending database migrations.");
        }
        catch (Exception exception) when (exception is not OperationCanceledException || !cancellationToken.IsCancellationRequested)
        {
            return HealthCheckResult.Unhealthy("PostgreSQL unavailable.");
        }
    }
}

public sealed class MqttHealthCheck(IMqttConnectionState mqtt) : IHealthCheck
{
    public Task<HealthCheckResult> CheckHealthAsync(HealthCheckContext context, CancellationToken cancellationToken = default) =>
        Task.FromResult(mqtt.IsConnected ? HealthCheckResult.Healthy() : HealthCheckResult.Degraded("MQTT cloud client disconnected."));
}

public sealed class BrokerReconciliationHealthCheck(BrokerReconciler reconciler) : IHealthCheck
{
    public Task<HealthCheckResult> CheckHealthAsync(HealthCheckContext context, CancellationToken cancellationToken = default) =>
        Task.FromResult(reconciler.HasCompletedOnce ? HealthCheckResult.Healthy() : HealthCheckResult.Degraded("Initial broker reconciliation pending."));
}

public sealed class BackgroundServicesHealthCheck(ServiceHeartbeats heartbeats) : IHealthCheck
{
    public Task<HealthCheckResult> CheckHealthAsync(HealthCheckContext context, CancellationToken cancellationToken = default)
    {
        var stalled = heartbeats.Stalled();
        return Task.FromResult(stalled.Count == 0
            ? HealthCheckResult.Healthy()
            : HealthCheckResult.Degraded($"Stalled: {string.Join(", ", stalled)}"));
    }
}

public static class HealthResponseWriter
{
    /// <summary>JSON con estado por check, sin excepciones ni cadenas de conexión.</summary>
    public static Task WriteAsync(HttpContext http, HealthReport report)
    {
        http.Response.ContentType = "application/json";
        var body = new
        {
            status = report.Status.ToString(),
            checks = report.Entries.ToDictionary(e => e.Key, e => new { status = e.Value.Status.ToString(), description = e.Value.Description }),
        };
        return http.Response.WriteAsync(JsonSerializer.Serialize(body));
    }
}
