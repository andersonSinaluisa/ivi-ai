using System.Text;
using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Jobs;
using Jarvis.Domain.Memory;
using Jarvis.Domain.Tasks;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Storage;
using Npgsql;

namespace Jarvis.Infrastructure.Persistence;

/// <summary>
/// DbContext de Jarvis. Convierte los eventos de dominio en filas de <c>events</c> dentro de la misma transacción,
/// serializa la escritura de eventos por owner (advisory lock) y notifica los eventos confirmados tras el commit.
/// </summary>
public sealed class JarvisDbContext : DbContext, IJarvisDbContext
{
    public const string Schema = "jarvis";
    public const int MaxEventDataBytes = 4096;

    private readonly IRequestContext? _requestContext;
    private readonly IPostCommitEventSink? _eventSink;
    private readonly TimeProvider _clock;
    private readonly List<DomainEvent> _standaloneEvents = [];
    private readonly List<EventRecord> _pendingNotifications = [];

    public JarvisDbContext(
        DbContextOptions<JarvisDbContext> options,
        IRequestContext? requestContext = null,
        IPostCommitEventSink? eventSink = null,
        TimeProvider? clock = null)
        : base(options)
    {
        _requestContext = requestContext;
        _eventSink = eventSink;
        _clock = clock ?? TimeProvider.System;
    }

    public DbSet<User> Users => Set<User>();

    public DbSet<RefreshToken> RefreshTokens => Set<RefreshToken>();

    public DbSet<Device> Devices => Set<Device>();

    public DbSet<DeviceCredential> DeviceCredentials => Set<DeviceCredential>();

    public DbSet<DeviceTaskGrant> DeviceTaskGrants => Set<DeviceTaskGrant>();

    public DbSet<JarvisTask> Tasks => Set<JarvisTask>();

    public DbSet<TaskApproval> TaskApprovals => Set<TaskApproval>();

    public DbSet<EventRecord> Events => Set<EventRecord>();

    public DbSet<MemoryItem> Memories => Set<MemoryItem>();

    public DbSet<MemoryVersion> MemoryVersions => Set<MemoryVersion>();

    public DbSet<MemoryRelation> MemoryRelations => Set<MemoryRelation>();

    public DbSet<MemoryEmbedding> MemoryEmbeddings => Set<MemoryEmbedding>();

    public DbSet<Experience> Experiences => Set<Experience>();

    public DbSet<Skill> Skills => Set<Skill>();

    public DbSet<SkillVersion> SkillVersions => Set<SkillVersion>();

    public DbSet<SkillProposal> SkillProposals => Set<SkillProposal>();

    public DbSet<SyncDevice> SyncDevices => Set<SyncDevice>();

    public DbSet<SyncReceipt> SyncReceipts => Set<SyncReceipt>();

    public DbSet<SyncConflict> SyncConflicts => Set<SyncConflict>();

    public DbSet<Job> Jobs => Set<Job>();

    public DbSet<JobRun> JobRuns => Set<JobRun>();

    // Usados por los filtros globales; EF los evalúa por consulta en esta instancia.
    internal bool BypassOwnerFilter => _requestContext is null || _requestContext.IsSystemScope;

    internal Guid CurrentOwnerId => _requestContext?.Principal?.OwnerId ?? Guid.Empty;

    public void AddEvent(DomainEvent domainEvent) => _standaloneEvents.Add(domainEvent);

    public override int SaveChanges(bool acceptAllChangesOnSuccess) =>
        throw new NotSupportedException("Use SaveChangesAsync.");

    public override async Task<int> SaveChangesAsync(bool acceptAllChangesOnSuccess, CancellationToken cancellationToken = default)
    {
        var records = CollectEvents();
        if (records.Count == 0)
        {
            return await base.SaveChangesAsync(acceptAllChangesOnSuccess, cancellationToken);
        }

        var ownsTransaction = Database.CurrentTransaction is null;
        var transaction = ownsTransaction ? await Database.BeginTransactionAsync(cancellationToken) : null;
        try
        {
            await LockOwnersAsync(records, cancellationToken);
            Events.AddRange(records);
            var written = await base.SaveChangesAsync(acceptAllChangesOnSuccess, cancellationToken);
            if (transaction is not null)
            {
                await transaction.CommitAsync(cancellationToken);
                _eventSink?.Publish(records);
            }
            else
            {
                _pendingNotifications.AddRange(records);
            }

            return written;
        }
        catch
        {
            foreach (var record in records)
            {
                Entry(record).State = EntityState.Detached;
            }

            throw;
        }
        finally
        {
            if (transaction is not null)
            {
                await transaction.DisposeAsync();
            }
        }
    }

    public async Task<T> InTransactionAsync<T>(Func<CancellationToken, Task<T>> action, CancellationToken cancellationToken)
    {
        if (Database.CurrentTransaction is not null)
        {
            return await action(cancellationToken);
        }

        await using var transaction = await Database.BeginTransactionAsync(cancellationToken);
        try
        {
            var result = await action(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
            if (_pendingNotifications.Count > 0)
            {
                _eventSink?.Publish(_pendingNotifications.ToList());
            }

            return result;
        }
        finally
        {
            _pendingNotifications.Clear();
        }
    }

    public void ResetTracking()
    {
        ChangeTracker.Clear();
        _standaloneEvents.Clear();
    }

    public bool IsUniqueViolation(Exception exception, string? constraintName = null)
    {
        for (var current = exception; current is not null; current = current.InnerException)
        {
            if (current is PostgresException { SqlState: PostgresErrorCodes.UniqueViolation } postgres)
            {
                return constraintName is null || postgres.ConstraintName == constraintName;
            }
        }

        return false;
    }

    public bool IsConcurrencyConflict(Exception exception) => exception is DbUpdateConcurrencyException;

    public Task<List<JarvisTask>> ClaimDispatchableTasksAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken) =>
        Tasks.FromSql($"""
            SELECT t.*, t.xmin
            FROM jarvis.tasks t
            JOIN jarvis.devices d ON d.id = t.target_device_id
            WHERE t.status = 'Pending'
              AND d.presence_state = 'Online'
              AND d.admin_state = 'Active'
              AND t.expires_at > {now}
            ORDER BY t.created_at
            LIMIT {batchSize}
            FOR UPDATE OF t SKIP LOCKED
            """)
            .IgnoreQueryFilters()
            .ToListAsync(cancellationToken);

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.HasDefaultSchema(Schema);
        modelBuilder.HasPostgresExtension("citext");
        modelBuilder.ApplyConfigurationsFromAssembly(typeof(JarvisDbContext).Assembly);

        modelBuilder.Entity<Device>().HasQueryFilter(d => BypassOwnerFilter || d.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<JarvisTask>().HasQueryFilter(t => BypassOwnerFilter || t.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<DeviceTaskGrant>().HasQueryFilter(g => BypassOwnerFilter || g.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<EventRecord>().HasQueryFilter(e => BypassOwnerFilter || e.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<DeviceCredential>().HasQueryFilter(c => BypassOwnerFilter || Devices.Any(d => d.Id == c.DeviceId && d.OwnerId == CurrentOwnerId));

        // Memoria (ADR-013): mismo aislamiento por owner.
        modelBuilder.HasSequence<long>(MemorySchema.ChangeSequence, MemorySchema.Name);
        modelBuilder.Entity<MemoryItem>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<MemoryVersion>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<MemoryRelation>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<MemoryEmbedding>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<Experience>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<Skill>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<SkillVersion>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<SkillProposal>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<SyncDevice>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<SyncReceipt>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<SyncConflict>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<Job>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);
        modelBuilder.Entity<JobRun>().HasQueryFilter(m => BypassOwnerFilter || m.OwnerId == CurrentOwnerId);

        SnakeCaseNaming.Apply(modelBuilder);
    }

    private List<EventRecord> CollectEvents()
    {
        var domainEvents = new List<DomainEvent>();
        foreach (var entry in ChangeTracker.Entries<Entity>().ToList())
        {
            domainEvents.AddRange(entry.Entity.DomainEvents);
            entry.Entity.ClearDomainEvents();
        }

        domainEvents.AddRange(_standaloneEvents);
        _standaloneEvents.Clear();
        if (domainEvents.Count == 0)
        {
            return [];
        }

        var recordedAt = _clock.GetUtcNow();
        var principal = _requestContext?.Principal;
        var ambientActor = principal is null ? Actor.System : principal.ToActor();
        var correlationId = _requestContext?.CorrelationId ?? Guid.CreateVersion7();

        return domainEvents.Select(e =>
        {
            var actor = e.Actor ?? ambientActor;
            var data = JsonSerializer.Serialize(e.Data ?? new Dictionary<string, object?>(), JarvisJson.Options);
            if (Encoding.UTF8.GetByteCount(data) > MaxEventDataBytes)
            {
                throw new InvalidOperationException($"Event {e.Type} data exceeds {MaxEventDataBytes} bytes.");
            }

            return new EventRecord
            {
                Id = Guid.CreateVersion7(recordedAt),
                OwnerId = e.OwnerId,
                Type = e.Type,
                OccurredAt = e.OccurredAt,
                RecordedAt = recordedAt,
                CorrelationId = e.CorrelationId ?? correlationId,
                ActorType = actor.Type,
                ActorId = actor.Id,
                DeviceId = e.DeviceId,
                TaskId = e.TaskId,
                Data = data,
            };
        }).ToList();
    }

    private async Task LockOwnersAsync(List<EventRecord> records, CancellationToken cancellationToken)
    {
        // Serializa la escritura de eventos por owner para que el orden de seq coincida con el de commit (catch-up).
        foreach (var ownerId in records.Where(r => r.OwnerId is not null).Select(r => r.OwnerId!.Value).Distinct().Order())
        {
            var key = ownerId.ToString("D");
            await Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtextextended({key}, 0))", cancellationToken);
        }
    }
}
