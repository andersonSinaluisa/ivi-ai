using Jarvis.Application.Abstractions;
using Jarvis.Application.Configuration;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Caching.Memory;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Security;

/// <summary>
/// Verifica que el principal de un token siga activo (user Active / device Active), con caché de 30 s.
/// Una revocación deja de aceptar tokens vigentes en como máximo el TTL de la caché.
/// </summary>
public sealed class PrincipalStatusService(IMemoryCache cache, IOptions<SecurityOptions> options)
{
    private readonly TimeSpan _ttl = TimeSpan.FromSeconds(options.Value.PrincipalStatusCacheSeconds);

    public async Task<bool> IsActiveAsync(Principal principal, IJarvisDbContext db, CancellationToken cancellationToken)
    {
        var key = CacheKey(principal.Id);
        if (cache.TryGetValue(key, out bool active))
        {
            return active;
        }

        active = principal.Type == PrincipalType.User
            ? await db.Users.AsNoTracking()
                .AnyAsync(u => u.Id == principal.Id && u.Status == UserStatus.Active, cancellationToken)
            : await db.Devices.IgnoreQueryFilters().AsNoTracking()
                .AnyAsync(d => d.Id == principal.Id && d.OwnerId == principal.OwnerId && d.AdminState == DeviceAdminState.Active, cancellationToken);
        cache.Set(key, active, _ttl);
        return active;
    }

    public void Invalidate(Guid principalId) => cache.Remove(CacheKey(principalId));

    private static string CacheKey(Guid id) => $"principal-status:{id:N}";
}
