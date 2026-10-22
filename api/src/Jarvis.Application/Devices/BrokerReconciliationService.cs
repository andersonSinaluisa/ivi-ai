using Jarvis.Application.Abstractions;
using Jarvis.Domain.Devices;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;

namespace Jarvis.Application.Devices;

/// <summary>
/// HU-019: hace converger Dynamic Security al estado de PostgreSQL.
/// Nunca recrea clients de devices (no conoce los secretos). Un client huérfano se elimina solo si lo fue
/// en dos ciclos consecutivos, para no borrar el client de un registro todavía no confirmado.
/// </summary>
public sealed class BrokerReconciler(IBrokerCredentialAdmin broker, ILogger<BrokerReconciler> logger)
{
    private readonly HashSet<string> _orphanCandidates = [];

    public bool HasCompletedOnce { get; private set; }

    /// <summary>Devuelve true si quedaron diferencias que requieren otro ciclo pronto.</summary>
    public async Task<bool> ReconcileAsync(IJarvisDbContext db, TimeProvider clock, CancellationToken cancellationToken)
    {
        var clients = (await broker.ListClientsAsync(cancellationToken))
            .Where(c => Guid.TryParseExact(c.Username, "D", out _))
            .ToDictionary(c => c.Username, StringComparer.Ordinal);
        var devices = await db.Devices.ToListAsync(cancellationToken);
        var byUsername = devices.ToDictionary(d => d.Id.ToString("D"), StringComparer.Ordinal);
        var pending = false;

        var orphans = clients.Keys
            .Where(u => !byUsername.TryGetValue(u, out var device) || device.AdminState == DeviceAdminState.Revoked)
            .ToHashSet(StringComparer.Ordinal);
        var newCandidates = new HashSet<string>(StringComparer.Ordinal);
        foreach (var username in orphans)
        {
            if (_orphanCandidates.Contains(username))
            {
                await broker.DeleteClientAsync(Guid.ParseExact(username, "D"), cancellationToken);
                logger.LogInformation("Deleted orphan broker client {Username}", username);
            }
            else
            {
                newCandidates.Add(username);
                pending = true;
            }
        }

        _orphanCandidates.Clear();
        _orphanCandidates.UnionWith(newCandidates);

        var now = clock.GetUtcNow();
        foreach (var device in devices.Where(d => d.AdminState != DeviceAdminState.Revoked))
        {
            if (!clients.TryGetValue(device.Id.ToString("D"), out var client))
            {
                if (device.BrokerSyncedAt is not null)
                {
                    device.MarkBrokerUnsynced();
                    logger.LogWarning("Device {DeviceId} has no broker client; its credential must be rotated", device.Id);
                }

                continue;
            }

            var shouldBeDisabled = device.AdminState == DeviceAdminState.Disabled;
            if (client.Disabled != shouldBeDisabled)
            {
                if (shouldBeDisabled)
                {
                    await broker.DisableClientAsync(device.Id, cancellationToken);
                }
                else
                {
                    await broker.EnableClientAsync(device.Id, cancellationToken);
                }

                logger.LogInformation("Broker client {DeviceId} set disabled={Disabled}", device.Id, shouldBeDisabled);
            }

            if (device.BrokerSyncedAt is null)
            {
                device.MarkBrokerSynced(now);
            }
        }

        await db.SaveChangesAsync(cancellationToken);
        HasCompletedOnce = true;
        return pending;
    }
}
