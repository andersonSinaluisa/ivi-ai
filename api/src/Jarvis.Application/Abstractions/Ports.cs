using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Jobs;
using Jarvis.Domain.Memory;
using Jarvis.Domain.Tasks;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Abstractions;

/// <summary>
/// Acceso a datos para los casos de uso. DbContext ya es Unit of Work + Repository (docs/01-architecture/backend-architecture.md).
/// </summary>
public interface IJarvisDbContext
{
    DbSet<User> Users { get; }

    DbSet<RefreshToken> RefreshTokens { get; }

    DbSet<Device> Devices { get; }

    DbSet<DeviceCredential> DeviceCredentials { get; }

    DbSet<DeviceTaskGrant> DeviceTaskGrants { get; }

    DbSet<JarvisTask> Tasks { get; }

    DbSet<TaskApproval> TaskApprovals { get; }

    DbSet<EventRecord> Events { get; }

    DbSet<MemoryItem> Memories { get; }

    DbSet<MemoryVersion> MemoryVersions { get; }

    DbSet<MemoryRelation> MemoryRelations { get; }

    DbSet<MemoryEmbedding> MemoryEmbeddings { get; }

    DbSet<Experience> Experiences { get; }

    DbSet<Skill> Skills { get; }

    DbSet<SkillVersion> SkillVersions { get; }

    DbSet<SkillProposal> SkillProposals { get; }

    DbSet<SyncDevice> SyncDevices { get; }

    DbSet<SyncReceipt> SyncReceipts { get; }

    DbSet<SyncConflict> SyncConflicts { get; }

    DbSet<Job> Jobs { get; }

    DbSet<JobRun> JobRuns { get; }

    /// <summary>Registra un evento que no pertenece a ninguna entidad rastreada (p. ej. TASK_REJECTED).</summary>
    void AddEvent(DomainEvent domainEvent);

    Task<int> SaveChangesAsync(CancellationToken cancellationToken = default);

    /// <summary>Ejecuta <paramref name="action"/> en una transacción; los eventos se notifican tras el commit.</summary>
    Task<T> InTransactionAsync<T>(Func<CancellationToken, Task<T>> action, CancellationToken cancellationToken);

    /// <summary>Descarta los cambios rastreados (tras un error de concurrencia o de unicidad).</summary>
    void ResetTracking();

    bool IsUniqueViolation(Exception exception, string? constraintName = null);

    bool IsConcurrencyConflict(Exception exception);

    /// <summary>Reclama tareas Pending de targets Online con FOR UPDATE SKIP LOCKED (debe llamarse dentro de una transacción).</summary>
    Task<List<JarvisTask>> ClaimDispatchableTasksAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken);
}

/// <summary>Consultas SQL específicas de la memoria y los jobs (secuencia de cambios, búsqueda, reclamo de ejecuciones).</summary>
public interface IMemorySql
{
    /// <summary>Serializa los cambios de sincronización de un owner (advisory lock de transacción; requiere transacción).</summary>
    Task LockOwnerChangesAsync(Guid ownerId, CancellationToken cancellationToken);

    /// <summary>Siguiente valor de la secuencia de cambios.</summary>
    Task<long> NextSeqAsync(CancellationToken cancellationToken);

    /// <summary>Mayor seq visible de un owner (cursor actual).</summary>
    Task<long> CurrentSeqAsync(Guid ownerId, CancellationToken cancellationToken);

    /// <summary>Registros de un owner cambiados después de <paramref name="since"/>, en orden de seq.</summary>
    Task<IReadOnlyList<ChangeRef>> ChangedSinceAsync(Guid ownerId, long since, int limit, IReadOnlyCollection<string> entities, IReadOnlyCollection<string> embeddingModels, CancellationToken cancellationToken);

    /// <summary>¿Está instalada la extensión pgvector?</summary>
    Task<bool> VectorExtensionAvailableAsync(CancellationToken cancellationToken);

    /// <summary>Búsqueda vectorial con pgvector (distancia coseno) entre vectores del mismo modelo y dimensión.</summary>
    Task<IReadOnlyList<ScoredId>> VectorSearchAsync(Guid ownerId, string model, float[] vector, int limit, CancellationToken cancellationToken);

    /// <summary>Búsqueda de texto (PostgreSQL full text, configuración simple) sobre resumen y contenido.</summary>
    Task<IReadOnlyList<ScoredId>> TextSearchAsync(Guid ownerId, IReadOnlyList<string> terms, int limit, CancellationToken cancellationToken);

    /// <summary>Búsqueda de texto sobre objetivos de experiencias.</summary>
    Task<IReadOnlyList<ScoredId>> ExperienceSearchAsync(Guid ownerId, IReadOnlyList<string> terms, int limit, CancellationToken cancellationToken);

    /// <summary>Reclama ejecuciones encoladas o con lease vencida (FOR UPDATE SKIP LOCKED; requiere transacción).</summary>
    Task<List<JobRun>> ClaimRunsAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken);

    /// <summary>Jobs con próxima ejecución vencida y sin ejecución activa (FOR UPDATE SKIP LOCKED; requiere transacción).</summary>
    Task<List<Job>> ClaimDueJobsAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken);
}

public sealed record ChangeRef(string Entity, Guid EntityId, string? Model, long Seq);

public sealed record ScoredId(Guid Id, double Score);

public enum PrincipalType
{
    User,
    Device,
}

public sealed record Principal(PrincipalType Type, Guid Id, Guid OwnerId)
{
    public Actor ToActor() => Type == PrincipalType.User ? Actor.User(Id) : Actor.Device(Id);

    public bool IsUser => Type == PrincipalType.User;

    public bool IsDevice => Type == PrincipalType.Device;
}

/// <summary>Contexto por scope: principal, correlation id y modo sistema (sin filtro por owner).</summary>
public interface IRequestContext
{
    Principal? Principal { get; set; }

    Guid CorrelationId { get; set; }

    bool IsSystemScope { get; }

    void EnterSystemScope();
}

public sealed class RequestContext : IRequestContext
{
    public Principal? Principal { get; set; }

    public Guid CorrelationId { get; set; } = Guid.CreateVersion7();

    public bool IsSystemScope { get; private set; }

    public void EnterSystemScope() => IsSystemScope = true;
}

public interface IDeviceMessagePublisher
{
    /// <summary>Publica QoS 1 sin retain en el topic down correspondiente. Devuelve false si no pudo publicar.</summary>
    Task<bool> PublishAsync<T>(
        Guid deviceId,
        string messageType,
        T data,
        Guid? correlationId,
        Guid? causationId,
        CancellationToken cancellationToken);
}

public sealed record BrokerClientInfo(string Username, bool Disabled, IReadOnlyList<string> Roles);

public interface IBrokerCredentialAdmin
{
    Task CreateClientAsync(Guid deviceId, string secret, CancellationToken cancellationToken);

    Task SetPasswordAsync(Guid deviceId, string secret, CancellationToken cancellationToken);

    Task DisableClientAsync(Guid deviceId, CancellationToken cancellationToken);

    Task EnableClientAsync(Guid deviceId, CancellationToken cancellationToken);

    Task DeleteClientAsync(Guid deviceId, CancellationToken cancellationToken);

    Task<IReadOnlyList<BrokerClientInfo>> ListClientsAsync(CancellationToken cancellationToken);
}

public class BrokerUnavailableException(string message, Exception? inner = null) : Exception(message, inner);

public class BrokerCommandException(string command, string error) : Exception($"Broker command '{command}' failed: {error}")
{
    public string Command { get; } = command;

    public string Error { get; } = error;

    public bool IsAlreadyExists => Error.Contains("already exists", StringComparison.OrdinalIgnoreCase);

    public bool IsNotFound => Error.Contains("not found", StringComparison.OrdinalIgnoreCase);
}

public interface IDeviceSecretHasher
{
    byte[] Hash(string secret);

    bool Verify(string secret, byte[] expectedHash);
}

public interface IUserPasswordHasher
{
    string Hash(string password);

    bool Verify(string passwordHash, string providedPassword);

    /// <summary>Ejecuta una verificación de costo equivalente para igualar tiempos cuando el user no existe.</summary>
    void VerifyAgainstDummy(string providedPassword);
}

public sealed record IssuedToken(string Token, DateTimeOffset ExpiresAt);

public interface ITokenIssuer
{
    IssuedToken IssueForUser(User user);

    IssuedToken IssueForDevice(Device device);
}

public interface IIpHasher
{
    string? Hash(string? ipAddress);
}

public interface IMqttConnectionState
{
    bool IsConnected { get; }
}

public interface IRetentionStore
{
    Task<int> DeleteEventsRecordedBeforeAsync(DateTimeOffset cutoff, int batchSize, CancellationToken cancellationToken);

    Task<int> DeleteRefreshTokensExpiredBeforeAsync(DateTimeOffset cutoff, int batchSize, CancellationToken cancellationToken);
}
