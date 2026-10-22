using Jarvis.Application.Abstractions;
using Jarvis.Application.Configuration;
using Jarvis.Application.Events;
using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Infrastructure.Persistence;
using Jarvis.IntegrationTests.Infrastructure;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;

namespace Jarvis.IntegrationTests;

/// <summary>HU-019 / HU-040.</summary>
public sealed class MaintenanceTests(JarvisTestEnvironment env, ITestOutputHelper output) : IntegrationTest(env, output)
{
    [Fact]
    public async Task Orphan_broker_clients_are_removed_by_reconciliation()
    {
        await OwnerAsync();
        var broker = Env.Services.GetRequiredService<IBrokerCredentialAdmin>();
        var orphan = Guid.CreateVersion7();
        await broker.CreateClientAsync(orphan, "jdv_orphan-secret-for-test", CancellationToken.None);
        Assert.Contains((await broker.ListClientsAsync(CancellationToken.None)), c => c.Username == orphan.ToString("D"));

        var reconciler = Env.Services.GetRequiredService<Jarvis.Application.Devices.BrokerReconciler>();
        await using var scope = Env.Services.CreateAsyncScope();
        scope.ServiceProvider.GetRequiredService<IRequestContext>().EnterSystemScope();
        var db = scope.ServiceProvider.GetRequiredService<IJarvisDbContext>();

        // Primer ciclo: solo lo marca como candidato (podría ser un registro aún sin commit). Segundo ciclo: lo borra.
        Assert.True(await reconciler.ReconcileAsync(db, TimeProvider.System, CancellationToken.None));
        Assert.Contains(await broker.ListClientsAsync(CancellationToken.None), c => c.Username == orphan.ToString("D"));
        await reconciler.ReconcileAsync(db, TimeProvider.System, CancellationToken.None);
        Assert.DoesNotContain(await broker.ListClientsAsync(CancellationToken.None), c => c.Username == orphan.ToString("D"));

        // Los devices legítimos no se tocan.
        var owner = await OwnerAsync();
        var device = await owner.RegisterDeviceAsync("Windows");
        await reconciler.ReconcileAsync(db, TimeProvider.System, CancellationToken.None);
        await reconciler.ReconcileAsync(db, TimeProvider.System, CancellationToken.None);
        Assert.Contains(await broker.ListClientsAsync(CancellationToken.None), c => c.Username == device.Device.Id.ToString("D"));
    }

    [Fact]
    public async Task Retention_deletes_only_old_events()
    {
        var owner = await OwnerAsync();
        var oldId = Guid.CreateVersion7();
        var recentId = Guid.CreateVersion7();
        await using (var scope = Env.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<JarvisDbContext>();
            foreach (var (id, recorded) in new[] { (oldId, DateTimeOffset.UtcNow.AddDays(-200)), (recentId, DateTimeOffset.UtcNow) })
            {
                await db.Database.ExecuteSqlAsync($$"""
                    INSERT INTO jarvis.events (id, owner_id, type, occurred_at, recorded_at, correlation_id, actor_type, data)
                    VALUES ({{id}}, {{owner.UserId}}, 'USER_CREATED', {{recorded}}, {{recorded}}, {{Guid.NewGuid()}}, 'System', '{}'::jsonb)
                    """);
            }

            var retention = new RetentionService(
                scope.ServiceProvider.GetRequiredService<IRetentionStore>(),
                TimeProvider.System,
                Options.Create(new RetentionOptions { EventsDays = 180, RefreshTokensDays = 30, BatchSize = 100 }),
                NullLogger<RetentionService>.Instance);
            await retention.RunAsync(CancellationToken.None);

            var remaining = await db.Events.IgnoreQueryFilters().Where(e => e.Id == oldId || e.Id == recentId).Select(e => e.Id).ToListAsync();
            Assert.Equal([recentId], remaining);
        }
    }
}
