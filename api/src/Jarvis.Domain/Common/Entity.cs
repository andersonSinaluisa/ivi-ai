namespace Jarvis.Domain.Common;

/// <summary>Base de entidades con identidad y eventos de dominio pendientes de persistir.</summary>
public abstract class Entity
{
    private readonly List<DomainEvent> _domainEvents = [];

    public Guid Id { get; protected set; }

    public IReadOnlyList<DomainEvent> DomainEvents => _domainEvents;

    protected void Raise(DomainEvent domainEvent) => _domainEvents.Add(domainEvent);

    public void ClearDomainEvents() => _domainEvents.Clear();
}

public enum ActorType
{
    User,
    Device,
    System,
}

public sealed record Actor(ActorType Type, Guid? Id)
{
    public static readonly Actor System = new(ActorType.System, null);

    public static Actor User(Guid id) => new(ActorType.User, id);

    public static Actor Device(Guid id) => new(ActorType.Device, id);
}

/// <summary>
/// Hecho de dominio que se persiste como fila de <c>events</c> en la misma transacción.
/// <see cref="Actor"/> y <see cref="CorrelationId"/> nulos se completan con el contexto ambiental al guardar.
/// </summary>
public sealed record DomainEvent(
    string Type,
    Guid? OwnerId,
    DateTimeOffset OccurredAt,
    Guid? DeviceId = null,
    Guid? TaskId = null,
    IReadOnlyDictionary<string, object?>? Data = null,
    Actor? Actor = null,
    Guid? CorrelationId = null);

public class DomainException(string code, string message) : Exception(message)
{
    public string Code { get; } = code;
}
