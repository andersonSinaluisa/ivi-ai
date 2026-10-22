using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Sync;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Sync;

/// <summary>
/// Permisos de sincronización (ADR-013): el owner (user) puede leer todo lo suyo; un device solo lo que sus scopes
/// permiten. Revocar o deshabilitar el device lo corta antes, en la autenticación (PrincipalStatusService).
/// </summary>
public sealed class SyncAccess(IJarvisDbContext db)
{
    public async Task<IReadOnlyList<string>> ScopesAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (principal.IsUser)
        {
            return SyncScopes.All;
        }

        var device = await db.SyncDevices.AsNoTracking().FirstOrDefaultAsync(d => d.DeviceId == principal.Id, cancellationToken);
        return device?.Scopes ?? [];
    }

    public async Task<AppError?> RequireAsync(Principal principal, string scope, CancellationToken cancellationToken)
    {
        var scopes = await ScopesAsync(principal, cancellationToken);
        return scopes.Contains(scope)
            ? null
            : new AppError(ErrorCodes.SyncScopeMissing, $"This device is not allowed to use '{scope}'.");
    }
}
