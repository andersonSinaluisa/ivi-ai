using System.Text;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace Jarvis.Infrastructure.Persistence;

// Mapeo según docs/04-data/database-design.md. Los nombres pasan a snake_case en SnakeCaseNaming.

internal sealed class UserConfiguration : IEntityTypeConfiguration<User>
{
    public void Configure(EntityTypeBuilder<User> builder)
    {
        builder.ToTable("users", t =>
        {
            t.HasCheckConstraint("ck_users_email_length", "length(email) <= 254");
            t.HasCheckConstraint("ck_users_display_name_length", "length(display_name) between 1 and 100");
            t.HasCheckConstraint("ck_users_status", "status in ('Active','Disabled')");
        });
        builder.HasKey(u => u.Id);
        builder.Property(u => u.Id).ValueGeneratedNever();
        builder.Property(u => u.Email).HasColumnType("citext").IsRequired();
        builder.HasIndex(u => u.Email).IsUnique().HasDatabaseName("ux_users_email");
        builder.Property(u => u.DisplayName).IsRequired();
        builder.Property(u => u.PasswordHash).IsRequired();
        builder.Property(u => u.Status).HasConversion<string>();
        builder.Ignore(u => u.DomainEvents);
    }
}

internal sealed class RefreshTokenConfiguration : IEntityTypeConfiguration<RefreshToken>
{
    public void Configure(EntityTypeBuilder<RefreshToken> builder)
    {
        builder.ToTable("refresh_tokens");
        builder.HasKey(t => t.Id);
        builder.Property(t => t.Id).ValueGeneratedNever();
        builder.Property(t => t.TokenHash).IsRequired();
        builder.HasIndex(t => t.TokenHash).IsUnique().HasDatabaseName("ux_refresh_tokens_token_hash");
        builder.HasIndex(t => t.UserId).HasDatabaseName("ix_refresh_tokens_user_id");
        builder.HasIndex(t => t.FamilyId).HasDatabaseName("ix_refresh_tokens_family_id");
        builder.HasOne<User>().WithMany().HasForeignKey(t => t.UserId).OnDelete(DeleteBehavior.Cascade);
        builder.HasOne<RefreshToken>().WithMany().HasForeignKey(t => t.ReplacedById).OnDelete(DeleteBehavior.SetNull);
    }
}

internal sealed class DeviceConfiguration : IEntityTypeConfiguration<Device>
{
    public void Configure(EntityTypeBuilder<Device> builder)
    {
        builder.ToTable("devices", t =>
        {
            t.HasCheckConstraint("ck_devices_name_length", "length(name) between 1 and 64");
            t.HasCheckConstraint("ck_devices_type", "type in ('Windows','ESP32','Mobile','Web','Unknown')");
            t.HasCheckConstraint("ck_devices_admin_state", "admin_state in ('Active','Disabled','Revoked')");
            t.HasCheckConstraint("ck_devices_presence_state", "presence_state in ('Online','Stale','Offline')");
            t.HasCheckConstraint("ck_devices_session_id_length", "current_session_id is null or length(current_session_id) <= 64");
            t.HasCheckConstraint("ck_devices_capabilities", "cardinality(capabilities) <= 32");
            t.HasCheckConstraint("ck_devices_supported_task_types", "cardinality(supported_task_types) <= 64");
            t.HasCheckConstraint("ck_devices_reported_info", "jsonb_typeof(reported_info) = 'object'");
            t.HasCheckConstraint("ck_devices_revoked_at", "admin_state <> 'Revoked' or revoked_at is not null");
        });
        builder.HasKey(d => d.Id);
        builder.Property(d => d.Id).ValueGeneratedNever();
        builder.Property(d => d.Name).IsRequired();
        builder.Property(d => d.Type).HasConversion<string>();
        builder.Property(d => d.AdminState).HasConversion<string>();
        builder.Property(d => d.PresenceState).HasConversion<string>();
        builder.Property(d => d.Capabilities).HasColumnType("text[]").IsRequired();
        builder.Property(d => d.SupportedTaskTypes).HasColumnType("text[]").IsRequired();
        builder.Property(d => d.ReportedInfo).HasColumnType("jsonb").IsRequired();
        builder.Property<uint>("xmin").HasColumnName("xmin").HasColumnType("xid").ValueGeneratedOnAddOrUpdate().IsConcurrencyToken();
        builder.HasOne<User>().WithMany().HasForeignKey(d => d.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(d => new { d.OwnerId, d.Name }).IsUnique().HasDatabaseName("ux_devices_owner_name");
        builder.HasIndex(d => new { d.PresenceState, d.LastSeenAt })
            .HasDatabaseName("ix_devices_presence_sweep")
            .HasFilter("presence_state <> 'Offline'");
        builder.HasIndex(d => d.Id).HasDatabaseName("ix_devices_broker_unsynced").HasFilter("broker_synced_at is null");
        builder.HasMany(d => d.Credentials).WithOne().HasForeignKey(c => c.DeviceId).OnDelete(DeleteBehavior.Restrict);
        builder.Navigation(d => d.Credentials).UsePropertyAccessMode(PropertyAccessMode.Field);
        builder.Ignore(d => d.Status);
        builder.Ignore(d => d.ActiveCredential);
        builder.Ignore(d => d.IsActive);
        builder.Ignore(d => d.DomainEvents);
    }
}

internal sealed class DeviceCredentialConfiguration : IEntityTypeConfiguration<DeviceCredential>
{
    public void Configure(EntityTypeBuilder<DeviceCredential> builder)
    {
        builder.ToTable("device_credentials", t =>
        {
            t.HasCheckConstraint("ck_device_credentials_kind", "kind in ('Secret','ClientCertificate')");
            t.HasCheckConstraint("ck_device_credentials_status", "status in ('Active','Revoked')");
            t.HasCheckConstraint(
                "ck_device_credentials_material",
                "(kind = 'Secret' and secret_hash is not null) or (kind = 'ClientCertificate' and certificate_thumbprint is not null)");
            t.HasCheckConstraint("ck_device_credentials_revoked_reason", "revoked_reason is null or length(revoked_reason) <= 200");
        });
        builder.HasKey(c => c.Id);
        builder.Property(c => c.Id).ValueGeneratedNever();
        builder.Property(c => c.Kind).HasConversion<string>();
        builder.Property(c => c.Status).HasConversion<string>();
        builder.HasIndex(c => c.DeviceId).IsUnique().HasDatabaseName("ux_device_credentials_active").HasFilter("status = 'Active'");
    }
}

internal sealed class DeviceTaskGrantConfiguration : IEntityTypeConfiguration<DeviceTaskGrant>
{
    public void Configure(EntityTypeBuilder<DeviceTaskGrant> builder)
    {
        builder.ToTable("device_task_grants", t =>
        {
            t.HasCheckConstraint("ck_grants_task_type", "task_type ~ '^[A-Z][A-Z0-9_]{1,63}$'");
            t.HasCheckConstraint("ck_grants_distinct_devices", "source_device_id <> target_device_id");
        });
        builder.HasKey(g => g.Id);
        builder.Property(g => g.Id).ValueGeneratedNever();
        builder.Property(g => g.TaskType).IsRequired();
        builder.HasOne<User>().WithMany().HasForeignKey(g => g.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<User>().WithMany().HasForeignKey(g => g.CreatedByUserId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<Device>().WithMany().HasForeignKey(g => g.SourceDeviceId).OnDelete(DeleteBehavior.Cascade);
        builder.HasOne<Device>().WithMany().HasForeignKey(g => g.TargetDeviceId).OnDelete(DeleteBehavior.Cascade);
        builder.HasIndex(g => new { g.SourceDeviceId, g.TargetDeviceId, g.TaskType }).IsUnique().HasDatabaseName("ux_grants");
        builder.HasIndex(g => g.TargetDeviceId).HasDatabaseName("ix_grants_target_device_id");
        builder.Ignore(g => g.DomainEvents);
    }
}

internal sealed class TaskApprovalConfiguration : IEntityTypeConfiguration<TaskApproval>
{
    public void Configure(EntityTypeBuilder<TaskApproval> builder)
    {
        builder.ToTable("task_approvals", t =>
        {
            t.HasCheckConstraint("ck_task_approvals_decision", "decision in ('Granted','Denied')");
            t.HasCheckConstraint("ck_task_approvals_decided_by_type", "decided_by_type in ('User','Device')");
            t.HasCheckConstraint(
                "ck_task_approvals_decider",
                "(decided_by_type = 'User' and decided_by_user_id is not null and decided_by_device_id is null) or "
                + "(decided_by_type = 'Device' and decided_by_device_id is not null and decided_by_user_id is null)");
            t.HasCheckConstraint("ck_task_approvals_signature", "signature_alg = 'Ed25519'");
        });
        builder.HasKey(a => a.Id);
        builder.Property(a => a.Id).ValueGeneratedNever();
        builder.Property(a => a.Decision).HasConversion<string>();
        builder.Property(a => a.DecidedByType).HasConversion<string>();
        builder.Property(a => a.Nonce).IsRequired().HasMaxLength(DomainRules.MaxNonceLength);
        builder.Property(a => a.SignatureAlg).IsRequired().HasMaxLength(16);
        builder.Property(a => a.SignatureKeyId).IsRequired().HasMaxLength(DomainRules.MaxKeyIdLength);
        builder.Property(a => a.SignatureValue).IsRequired().HasMaxLength(128);
        builder.Ignore(a => a.Signature);

        builder.HasOne<User>().WithMany().HasForeignKey(a => a.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<JarvisTask>().WithMany().HasForeignKey(a => a.TaskId).OnDelete(DeleteBehavior.Cascade);
        builder.HasOne<User>().WithMany().HasForeignKey(a => a.DecidedByUserId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<Device>().WithMany().HasForeignKey(a => a.DecidedByDeviceId).OnDelete(DeleteBehavior.Restrict);

        // Una sola decisión por tarea: la primera que llega es la que se transmite.
        builder.HasIndex(a => a.TaskId).IsUnique().HasDatabaseName("ux_task_approvals_task");
        // Un nonce no se reutiliza dentro del owner; el agente lo vuelve a comprobar por su cuenta.
        builder.HasIndex(a => new { a.OwnerId, a.Nonce }).IsUnique().HasDatabaseName("ux_task_approvals_nonce");
        builder.Ignore(a => a.DomainEvents);
    }
}

internal sealed class JarvisTaskConfiguration : IEntityTypeConfiguration<JarvisTask>
{
    public void Configure(EntityTypeBuilder<JarvisTask> builder)
    {
        builder.ToTable("tasks", t =>
        {
            t.HasCheckConstraint("ck_tasks_requested_by_type", "requested_by_type in ('User','Device')");
            t.HasCheckConstraint(
                "ck_tasks_requester",
                "(requested_by_type = 'User' and requested_by_user_id is not null and source_device_id is null) or "
                + "(requested_by_type = 'Device' and source_device_id is not null and requested_by_user_id is null)");
            t.HasCheckConstraint("ck_tasks_type", "type ~ '^[A-Z][A-Z0-9_]{1,63}$'");
            t.HasCheckConstraint("ck_tasks_payload_object", "jsonb_typeof(payload) = 'object'");
            t.HasCheckConstraint("ck_tasks_payload_size", "pg_column_size(payload) <= 16384");
            t.HasCheckConstraint("ck_tasks_result_size", "result is null or pg_column_size(result) <= 32768");
            t.HasCheckConstraint(
                "ck_tasks_status",
                "status in ('Pending','Dispatched','Delivered','Running','WaitingForApproval','Completed','Failed','Cancelled','Expired')");
            t.HasCheckConstraint("ck_tasks_terminal_completed_at", "status not in ('Completed','Failed','Cancelled','Expired') or completed_at is not null");
            t.HasCheckConstraint("ck_tasks_completed_result", "status <> 'Completed' or result is not null");
            t.HasCheckConstraint("ck_tasks_failed_error", "status not in ('Failed','Cancelled','Expired') or error_code is not null");
            t.HasCheckConstraint("ck_tasks_idempotency_key", "idempotency_key is null or length(idempotency_key) between 1 and 100");
            t.HasCheckConstraint("ck_tasks_max_duration", "max_duration_seconds is null or max_duration_seconds between 1 and 86400");
            t.HasCheckConstraint("ck_tasks_progress_percent", "progress_percent is null or progress_percent between 0 and 100");
            t.HasCheckConstraint("ck_tasks_progress_message", "progress_message is null or length(progress_message) <= 200");
            t.HasCheckConstraint("ck_tasks_error_code", "error_code is null or length(error_code) <= 64");
            t.HasCheckConstraint("ck_tasks_error_message", "error_message is null or length(error_message) <= 500");
            t.HasCheckConstraint("ck_tasks_status_reason", "status_reason is null or length(status_reason) <= 100");
            // La firma es opcional, pero o están las tres columnas o no está ninguna (ADR-012).
            t.HasCheckConstraint(
                "ck_tasks_signature_complete",
                "(signature_alg is null and signature_key_id is null and signature_value is null) or "
                + "(signature_alg = 'Ed25519' and signature_key_id is not null and signature_value is not null)");
        });
        builder.HasKey(t => t.Id);
        builder.Property(t => t.Id).ValueGeneratedNever();
        builder.Property(t => t.RequestedByType).HasConversion<string>();
        builder.Property(t => t.Status).HasConversion<string>();
        builder.Property(t => t.Type).IsRequired();
        builder.Property(t => t.Payload).HasColumnType("jsonb").IsRequired();
        builder.Property(t => t.Result).HasColumnType("jsonb");
        builder.Property<uint>("xmin").HasColumnName("xmin").HasColumnType("xid").ValueGeneratedOnAddOrUpdate().IsConcurrencyToken();
        builder.Property(t => t.SignatureAlg).HasMaxLength(16);
        builder.Property(t => t.SignatureKeyId).HasMaxLength(DomainRules.MaxKeyIdLength);
        builder.Property(t => t.SignatureValue).HasMaxLength(128);
        builder.Ignore(t => t.Signature);
        builder.Ignore(t => t.IsSigned);

        builder.HasOne<User>().WithMany().HasForeignKey(t => t.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<User>().WithMany().HasForeignKey(t => t.RequestedByUserId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<Device>().WithMany().HasForeignKey(t => t.SourceDeviceId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<Device>().WithMany().HasForeignKey(t => t.TargetDeviceId).OnDelete(DeleteBehavior.Restrict);

        builder.HasIndex(t => new { t.TargetDeviceId, t.CreatedAt }).HasDatabaseName("ix_tasks_dispatch").HasFilter("status = 'Pending'");
        builder.HasIndex(t => t.DispatchedAt).HasDatabaseName("ix_tasks_ack_timeout").HasFilter("status = 'Dispatched'");
        builder.HasIndex(t => t.ExpiresAt).HasDatabaseName("ix_tasks_expiry").HasFilter("status in ('Pending','Dispatched')");
        builder.HasIndex(t => t.TargetDeviceId)
            .HasDatabaseName("ix_tasks_active_by_target")
            .HasFilter("status in ('Dispatched','Delivered','Running','WaitingForApproval')");
        builder.HasIndex(t => new { t.OwnerId, t.CreatedAt, t.Id })
            .HasDatabaseName("ix_tasks_owner_created")
            .IsDescending(false, true, true);
        builder.HasIndex(t => new { t.SourceDeviceId, t.CreatedAt })
            .HasDatabaseName("ix_tasks_source_created")
            .IsDescending(false, true)
            .HasFilter("source_device_id is not null");
        builder.HasIndex(t => t.CorrelationId).HasDatabaseName("ix_tasks_correlation");

        // Equivalente al índice con COALESCE del diseño: NULLS NOT DISTINCT trata user/device nulos como iguales.
        builder.HasIndex(t => new { t.OwnerId, t.RequestedByType, t.RequestedByUserId, t.SourceDeviceId, t.IdempotencyKey })
            .IsUnique()
            .AreNullsDistinct(false)
            .HasDatabaseName("ux_tasks_idempotency")
            .HasFilter("idempotency_key is not null");

        builder.Ignore(t => t.IsTerminal);
        builder.Ignore(t => t.Requester);
        builder.Ignore(t => t.ExecutionDeadline);
        builder.Ignore(t => t.DomainEvents);
    }
}

internal sealed class EventRecordConfiguration : IEntityTypeConfiguration<EventRecord>
{
    public void Configure(EntityTypeBuilder<EventRecord> builder)
    {
        builder.ToTable("events", t =>
        {
            t.HasCheckConstraint("ck_events_type", "type ~ '^[A-Z][A-Z0-9_]{2,63}$'");
            t.HasCheckConstraint("ck_events_actor_type", "actor_type in ('User','Device','System')");
            t.HasCheckConstraint("ck_events_data_size", "pg_column_size(data) <= 4096");
        });
        builder.HasKey(e => e.Seq);
        builder.Property(e => e.Seq).UseIdentityAlwaysColumn();
        builder.HasIndex(e => e.Id).IsUnique().HasDatabaseName("ux_events_id");
        builder.Property(e => e.Type).IsRequired();
        builder.Property(e => e.ActorType).HasConversion<string>();
        builder.Property(e => e.Data).HasColumnType("jsonb").IsRequired();
        builder.Property(e => e.RecordedAt).HasDefaultValueSql("now()");
        builder.HasOne<User>().WithMany().HasForeignKey(e => e.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(e => new { e.OwnerId, e.Seq }).HasDatabaseName("ix_events_owner_seq");
        builder.HasIndex(e => new { e.TaskId, e.Seq }).HasDatabaseName("ix_events_task").HasFilter("task_id is not null");
        builder.HasIndex(e => new { e.DeviceId, e.Seq }).HasDatabaseName("ix_events_device").HasFilter("device_id is not null");
        builder.HasIndex(e => e.CorrelationId).HasDatabaseName("ix_events_correlation");
        builder.HasIndex(e => e.RecordedAt).HasDatabaseName("ix_events_recorded_at");
    }
}

/// <summary>Convierte tablas, columnas, claves e índices a snake_case (docs/04-data/database-design.md#convenciones).</summary>
public static class SnakeCaseNaming
{
    public static void Apply(ModelBuilder modelBuilder)
    {
        foreach (var entity in modelBuilder.Model.GetEntityTypes())
        {
            var table = entity.GetTableName()!;
            entity.SetTableName(ToSnake(table));
            foreach (var property in entity.GetProperties())
            {
                if (property.GetColumnName() != "xmin")
                {
                    property.SetColumnName(ToSnake(property.Name));
                }
            }

            foreach (var key in entity.GetKeys())
            {
                key.SetName(key.IsPrimaryKey() ? $"pk_{ToSnake(table)}" : ToSnake(key.GetName()!));
            }

            foreach (var foreignKey in entity.GetForeignKeys())
            {
                var columns = string.Join("_", foreignKey.Properties.Select(p => ToSnake(p.Name)));
                foreignKey.SetConstraintName($"fk_{ToSnake(table)}_{columns}");
            }

            foreach (var index in entity.GetIndexes())
            {
                if (index.GetDatabaseName() is { } name && !name.StartsWith("ix_", StringComparison.Ordinal) && !name.StartsWith("ux_", StringComparison.Ordinal))
                {
                    index.SetDatabaseName($"ix_{ToSnake(table)}_{string.Join("_", index.Properties.Select(p => ToSnake(p.Name)))}");
                }
            }
        }
    }

    public static string ToSnake(string name)
    {
        var builder = new StringBuilder(name.Length + 8);
        for (var i = 0; i < name.Length; i++)
        {
            var c = name[i];
            if (char.IsUpper(c))
            {
                if (i > 0 && name[i - 1] != '_' && (char.IsLower(name[i - 1]) || (i + 1 < name.Length && char.IsLower(name[i + 1]))))
                {
                    builder.Append('_');
                }

                builder.Append(char.ToLowerInvariant(c));
            }
            else
            {
                builder.Append(c);
            }
        }

        return builder.ToString();
    }
}
