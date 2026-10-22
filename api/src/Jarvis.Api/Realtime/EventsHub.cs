using Jarvis.Api.Http;
using Jarvis.Application.Realtime;
using Jarvis.Contracts.Rest;
using Jarvis.Infrastructure.Security;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.SignalR;

namespace Jarvis.Api.Realtime;

/// <summary>
/// HU-041: hub de notificaciones para clientes UI. No expone métodos que cambien estado.
/// Cada conexión entra al group de su owner; el cliente no elige groups.
/// </summary>
[Authorize(Policy = Policies.UserOnly)]
public sealed class EventsHub : Hub
{
    public const string Path = "/hubs/events";
    public const string ClientMethod = "event";

    public static string OwnerGroup(Guid ownerId) => $"owner:{ownerId:D}";

    public override async Task OnConnectedAsync()
    {
        if (!Guid.TryParse(Context.User?.FindFirst(JarvisClaims.OwnerId)?.Value, out var ownerId))
        {
            Context.Abort();
            return;
        }

        await Groups.AddToGroupAsync(Context.ConnectionId, OwnerGroup(ownerId));
        await base.OnConnectedAsync();
    }
}

/// <summary>HU-042: envía notificaciones al group del owner. Un fallo nunca afecta al caso de uso.</summary>
public sealed class SignalRRealtimeNotifier(IHubContext<EventsHub> hub, ILogger<SignalRRealtimeNotifier> logger) : IRealtimeNotifier
{
    public async Task NotifyAsync(Guid ownerId, RealtimeNotification notification, CancellationToken cancellationToken)
    {
        try
        {
            await hub.Clients.Group(EventsHub.OwnerGroup(ownerId)).SendAsync(EventsHub.ClientMethod, notification, cancellationToken);
        }
        catch (Exception exception) when (exception is not OperationCanceledException)
        {
            logger.LogWarning(exception, "Realtime notification {Name} could not be sent", notification.Name);
        }
    }
}
