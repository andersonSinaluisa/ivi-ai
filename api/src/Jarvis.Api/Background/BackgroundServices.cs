using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Devices;
using Jarvis.Application.Events;
using Jarvis.Application.Presence;
using Jarvis.Application.Tasks;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Tasks;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;

namespace Jarvis.Api.Background;

/// <summary>
/// Base de los jobs periódicos: un scope por ciclo en modo sistema, latido para readiness y errores registrados sin
/// detener el servicio. PostgreSQL es la coordinación; no hay estado crítico en memoria.
/// </summary>
public abstract class PeriodicJob(IServiceScopeFactory scopes, ServiceHeartbeats heartbeats, ILogger logger) : BackgroundService
{
    protected abstract string Name { get; }

    protected abstract TimeSpan Interval { get; }

    protected ILogger Logger { get; } = logger;

    protected abstract Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken);

    /// <summary>Espera entre ciclos; los jobs pueden despertarse antes (p. ej. por señal).</summary>
    protected virtual Task WaitAsync(CancellationToken cancellationToken) => Task.Delay(Interval, cancellationToken);

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        await Task.Yield();
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                await using var scope = scopes.CreateAsyncScope();
                scope.ServiceProvider.GetRequiredService<IRequestContext>().EnterSystemScope();
                await RunOnceAsync(scope.ServiceProvider, stoppingToken);
                heartbeats.Beat(Name, Interval);
            }
            catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
            {
                break;
            }
            catch (Exception exception)
            {
                Logger.LogError(exception, "{Job} cycle failed", Name);
            }

            try
            {
                await WaitAsync(stoppingToken);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }
    }
}

/// <summary>HU-028: despierta por señal (creación de tarea, hello) o cada 5 s.</summary>
public sealed class TaskDispatcherService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    DispatchSignal signal,
    IOptions<TaskOptions> options,
    ILogger<TaskDispatcherService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    private DateTimeOffset _lastCancelResend = DateTimeOffset.MinValue;

    protected override string Name => "task-dispatcher";

    protected override TimeSpan Interval => TimeSpan.FromSeconds(options.Value.DispatchIntervalSeconds);

    protected override Task WaitAsync(CancellationToken cancellationToken) => signal.WaitAsync(Interval, cancellationToken);

    protected override async Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken)
    {
        var dispatcher = services.GetRequiredService<TaskDispatchService>();
        while (await dispatcher.DispatchPendingAsync(cancellationToken) >= options.Value.DispatchBatchSize)
        {
            // Lote lleno: puede haber más tareas listas.
        }

        var now = DateTimeOffset.UtcNow;
        if (now - _lastCancelResend >= TimeSpan.FromSeconds(options.Value.CancelResendSeconds))
        {
            _lastCancelResend = now;
            await dispatcher.ResendPendingCancellationsAsync(cancellationToken);
        }
    }
}

public sealed class DeliveryTimeoutSweeperService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    IOptions<TaskOptions> options,
    ILogger<DeliveryTimeoutSweeperService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    protected override string Name => "delivery-timeout-sweeper";

    protected override TimeSpan Interval => TimeSpan.FromSeconds(options.Value.DeliveryTimeoutSweepSeconds);

    protected override Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken) =>
        services.GetRequiredService<TaskSweepService>().SweepAckTimeoutsAsync(cancellationToken);
}

public sealed class TaskExpirySweeperService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    IOptions<TaskOptions> options,
    ILogger<TaskExpirySweeperService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    protected override string Name => "task-expiry-sweeper";

    protected override TimeSpan Interval => TimeSpan.FromSeconds(options.Value.ExpirySweepSeconds);

    protected override Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken) =>
        services.GetRequiredService<TaskSweepService>().SweepExpirationsAsync(cancellationToken);
}

/// <summary>HU-037 + actualización de gauges (tareas pendientes, devices por presencia).</summary>
public sealed class PresenceSweeperService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    JarvisMetrics metrics,
    IOptions<PresenceOptions> options,
    ILogger<PresenceSweeperService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    protected override string Name => "presence-sweeper";

    protected override TimeSpan Interval => TimeSpan.FromSeconds(options.Value.SweepIntervalSeconds);

    protected override async Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken)
    {
        await services.GetRequiredService<PresenceSweepService>().SweepAsync(cancellationToken);

        var db = services.GetRequiredService<IJarvisDbContext>();
        var byPresence = await db.Devices.AsNoTracking()
            .Where(d => d.AdminState != DeviceAdminState.Revoked)
            .GroupBy(d => d.PresenceState)
            .Select(g => new { g.Key, Count = g.LongCount() })
            .ToListAsync(cancellationToken);
        metrics.SetDevicesByPresence(byPresence.ToDictionary(x => x.Key.ToString(), x => x.Count));
        metrics.SetPendingTasks(await db.Tasks.LongCountAsync(t => t.Status == JarvisTaskStatus.Pending, cancellationToken));
    }
}

/// <summary>HU-019: al arrancar, cada hora y cada 30 s mientras haya diferencias o errores.</summary>
public sealed class BrokerReconciliationHostedService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    BrokerReconciler reconciler,
    IMqttConnectionState mqtt,
    TimeProvider clock,
    IOptions<BrokerReconciliationOptions> options,
    ILogger<BrokerReconciliationHostedService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    private bool _retrySoon = true;

    protected override string Name => "broker-reconciliation";

    protected override TimeSpan Interval => TimeSpan.FromMinutes(options.Value.IntervalMinutes);

    protected override Task WaitAsync(CancellationToken cancellationToken) =>
        Task.Delay(_retrySoon ? TimeSpan.FromSeconds(options.Value.RetrySeconds) : Interval, cancellationToken);

    protected override async Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken)
    {
        _retrySoon = true;
        for (var waited = 0; !mqtt.IsConnected && waited < 30; waited++)
        {
            // Al arrancar, la conexión MQTT suele tardar unos segundos: se espera antes de posponer.
            await Task.Delay(TimeSpan.FromSeconds(1), cancellationToken);
        }

        if (!mqtt.IsConnected)
        {
            Logger.LogInformation("Broker reconciliation postponed: MQTT not connected");
            return;
        }

        _retrySoon = await reconciler.ReconcileAsync(services.GetRequiredService<IJarvisDbContext>(), clock, cancellationToken);
    }
}

/// <summary>HU-040.</summary>
public sealed class RetentionHostedService(
    IServiceScopeFactory scopes,
    ServiceHeartbeats heartbeats,
    IOptions<RetentionOptions> options,
    ILogger<RetentionHostedService> logger) : PeriodicJob(scopes, heartbeats, logger)
{
    protected override string Name => "retention";

    protected override TimeSpan Interval => TimeSpan.FromMinutes(options.Value.IntervalMinutes);

    protected override Task RunOnceAsync(IServiceProvider services, CancellationToken cancellationToken) =>
        options.Value.Enabled ? services.GetRequiredService<RetentionService>().RunAsync(cancellationToken) : Task.CompletedTask;
}

/// <summary>Entrega en orden los eventos confirmados a los suscriptores (realtime, device origen, métricas).</summary>
public sealed class PostCommitEventPump(
    PostCommitEventChannel channel,
    IServiceScopeFactory scopes,
    ILogger<PostCommitEventPump> logger) : BackgroundService
{
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        await foreach (var record in channel.Reader.ReadAllAsync(stoppingToken))
        {
            await using var scope = scopes.CreateAsyncScope();
            var context = scope.ServiceProvider.GetRequiredService<IRequestContext>();
            context.EnterSystemScope();
            context.CorrelationId = record.CorrelationId;
            foreach (var subscriber in scope.ServiceProvider.GetServices<IEventSubscriber>())
            {
                try
                {
                    await subscriber.HandleAsync(record, stoppingToken);
                }
                catch (Exception exception) when (exception is not OperationCanceledException)
                {
                    logger.LogWarning(exception, "Event subscriber {Subscriber} failed for {EventType}", subscriber.GetType().Name, record.Type);
                }
            }
        }
    }
}
