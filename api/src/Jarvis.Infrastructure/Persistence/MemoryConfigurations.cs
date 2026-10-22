using Jarvis.Domain.Devices;
using Jarvis.Domain.Jobs;
using Jarvis.Domain.Memory;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;

namespace Jarvis.Infrastructure.Persistence;

// Esquema "memory" (ADR-013, ADR-015, ADR-016). Los nombres pasan a snake_case en SnakeCaseNaming.

internal static class MemorySchema
{
    public const string Name = "memory";
    public const string ChangeSequence = "change_seq";

    public static string InList(IEnumerable<string> values) => string.Join(",", values.Select(v => $"'{v}'"));
}

internal sealed class MemoryItemConfiguration : IEntityTypeConfiguration<MemoryItem>
{
    public void Configure(EntityTypeBuilder<MemoryItem> builder)
    {
        builder.ToTable("memories", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_memories_kind", $"kind in ({MemorySchema.InList(MemoryKinds.All)})");
            t.HasCheckConstraint("ck_memories_verification", $"verification in ({MemorySchema.InList(Verifications.All)})");
            t.HasCheckConstraint("ck_memories_retention", "retention_kind in ('permanent','days') and (retention_kind <> 'days' or retention_days between 1 and 36500)");
            t.HasCheckConstraint("ck_memories_content", "deleted_at is not null or (content is not null and length(content) between 1 and 8000)");
            t.HasCheckConstraint("ck_memories_summary", "length(summary) <= 500");
            t.HasCheckConstraint("ck_memories_tags", "cardinality(tags) <= 32");
            t.HasCheckConstraint("ck_memories_scores", "relevance between 0 and 1 and confidence between 0 and 1");
            t.HasCheckConstraint("ck_memories_version", "version >= 1");
        });
        builder.HasKey(m => m.Id);
        builder.Property(m => m.Id).ValueGeneratedNever();
        builder.Property(m => m.Kind).IsRequired();
        builder.Property(m => m.Summary).IsRequired();
        builder.Property(m => m.Tags).IsRequired();
        builder.Property(m => m.SourceKind).IsRequired();
        builder.Property(m => m.SourceLabel).IsRequired();
        builder.Ignore(m => m.IsDeleted);
        builder.HasOne<User>().WithMany().HasForeignKey(m => m.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<Device>().WithMany().HasForeignKey(m => m.OriginDeviceId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(m => new { m.OwnerId, m.Seq }).HasDatabaseName("ix_memories_owner_seq");
        builder.HasIndex(m => new { m.OwnerId, m.UpdatedAt }).HasDatabaseName("ix_memories_owner_updated").HasFilter("deleted_at is null");
    }
}

internal sealed class MemoryVersionConfiguration : IEntityTypeConfiguration<MemoryVersion>
{
    public void Configure(EntityTypeBuilder<MemoryVersion> builder)
    {
        builder.ToTable("memory_versions", MemorySchema.Name, t => t.HasCheckConstraint("ck_memory_versions_data", "jsonb_typeof(data) = 'object'"));
        builder.HasKey(v => new { v.MemoryId, v.Version });
        builder.Property(v => v.Data).HasColumnType("jsonb").IsRequired();
        builder.HasOne<MemoryItem>().WithMany().HasForeignKey(v => v.MemoryId).OnDelete(DeleteBehavior.Cascade);
        builder.HasIndex(v => v.OwnerId).HasDatabaseName("ix_memory_versions_owner");
    }
}

internal sealed class MemoryRelationConfiguration : IEntityTypeConfiguration<MemoryRelation>
{
    public void Configure(EntityTypeBuilder<MemoryRelation> builder)
    {
        builder.ToTable("memory_relations", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_memory_relations_kind", $"kind in ({MemorySchema.InList(RelationKinds.All)})");
            t.HasCheckConstraint("ck_memory_relations_provenance", $"provenance in ({MemorySchema.InList(Provenances.All)})");
            t.HasCheckConstraint("ck_memory_relations_confidence", "confidence between 0 and 1");
            t.HasCheckConstraint("ck_memory_relations_inferred_unverified", "provenance <> 'inferred' or not verified");
            t.HasCheckConstraint("ck_memory_relations_distinct", "source_id <> target_id");
            t.HasCheckConstraint("ck_memory_relations_note", "note is null or length(note) <= 500");
        });
        builder.HasKey(r => r.Id);
        builder.Property(r => r.Id).ValueGeneratedNever();
        builder.HasOne<MemoryItem>().WithMany().HasForeignKey(r => r.SourceId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<MemoryItem>().WithMany().HasForeignKey(r => r.TargetId).OnDelete(DeleteBehavior.Restrict);
        builder.HasOne<User>().WithMany().HasForeignKey(r => r.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(r => new { r.OwnerId, r.Seq }).HasDatabaseName("ix_memory_relations_owner_seq");
        builder.HasIndex(r => r.SourceId).HasDatabaseName("ix_memory_relations_source").HasFilter("deleted_at is null");
        builder.HasIndex(r => r.TargetId).HasDatabaseName("ix_memory_relations_target").HasFilter("deleted_at is null");
    }
}

internal sealed class MemoryEmbeddingConfiguration : IEntityTypeConfiguration<MemoryEmbedding>
{
    public void Configure(EntityTypeBuilder<MemoryEmbedding> builder)
    {
        builder.ToTable("memory_embeddings", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_memory_embeddings_dims", "dims between 1 and 4096 and cardinality(vector) = dims");
            t.HasCheckConstraint("ck_memory_embeddings_model", "length(model) between 1 and 128");
        });
        builder.HasKey(e => new { e.MemoryId, e.Model });
        builder.Property(e => e.Vector).HasColumnType("real[]").IsRequired();
        builder.HasOne<MemoryItem>().WithMany().HasForeignKey(e => e.MemoryId).OnDelete(DeleteBehavior.Cascade);
        builder.HasIndex(e => new { e.OwnerId, e.Model, e.Dims }).HasDatabaseName("ix_memory_embeddings_owner_model");
        builder.HasIndex(e => new { e.OwnerId, e.Seq }).HasDatabaseName("ix_memory_embeddings_owner_seq");
    }
}

internal sealed class ExperienceConfiguration : IEntityTypeConfiguration<Experience>
{
    public void Configure(EntityTypeBuilder<Experience> builder)
    {
        builder.ToTable("experiences", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_experiences_goal", "deleted_at is not null or length(goal) between 1 and 2000");
            t.HasCheckConstraint("ck_experiences_steps", "jsonb_typeof(steps) = 'array'");
            t.HasCheckConstraint("ck_experiences_size", "pg_column_size(steps) + pg_column_size(evidence) + pg_column_size(errors) <= 65536");
            t.HasCheckConstraint("ck_experiences_occurrences", "occurrences >= 1");
        });
        builder.HasKey(e => e.Id);
        builder.Property(e => e.Id).ValueGeneratedNever();
        builder.Property(e => e.Steps).HasColumnType("jsonb").IsRequired();
        builder.Property(e => e.Evidence).HasColumnType("jsonb").IsRequired();
        builder.Property(e => e.Errors).HasColumnType("jsonb").IsRequired();
        builder.HasOne<User>().WithMany().HasForeignKey(e => e.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(e => new { e.OwnerId, e.Seq }).HasDatabaseName("ix_experiences_owner_seq");
        builder.HasIndex(e => new { e.OwnerId, e.Signature }).HasDatabaseName("ix_experiences_owner_signature").HasFilter("deleted_at is null");
    }
}

internal sealed class SkillConfiguration : IEntityTypeConfiguration<Skill>
{
    public void Configure(EntityTypeBuilder<Skill> builder)
    {
        builder.ToTable("skills", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_skills_status", $"status in ({MemorySchema.InList(SkillStatuses.All)})");
            t.HasCheckConstraint("ck_skills_name", "deleted_at is not null or length(name) between 1 and 200");
            t.HasCheckConstraint("ck_skills_procedure", "length(procedure) <= 20000");
            t.HasCheckConstraint("ck_skills_verified", "status <> 'verified' or verified_at is not null");
        });
        builder.HasKey(s => s.Id);
        builder.Property(s => s.Id).ValueGeneratedNever();
        builder.Property(s => s.Compatibility).HasColumnType("jsonb").IsRequired();
        builder.Property(s => s.Permissions).HasColumnType("jsonb").IsRequired();
        builder.HasOne<User>().WithMany().HasForeignKey(s => s.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(s => new { s.OwnerId, s.Seq }).HasDatabaseName("ix_skills_owner_seq");
    }
}

internal sealed class SkillVersionConfiguration : IEntityTypeConfiguration<SkillVersion>
{
    public void Configure(EntityTypeBuilder<SkillVersion> builder)
    {
        builder.ToTable("skill_versions", MemorySchema.Name);
        builder.HasKey(v => new { v.SkillId, v.Number });
        builder.HasOne<Skill>().WithMany().HasForeignKey(v => v.SkillId).OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class SkillProposalConfiguration : IEntityTypeConfiguration<SkillProposal>
{
    public void Configure(EntityTypeBuilder<SkillProposal> builder)
    {
        builder.ToTable("skill_proposals", MemorySchema.Name, t =>
            t.HasCheckConstraint("ck_skill_proposals_status", "status in ('pending','accepted','rejected')"));
        builder.HasKey(p => p.Id);
        builder.Property(p => p.Id).ValueGeneratedNever();
        builder.HasOne<User>().WithMany().HasForeignKey(p => p.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(p => new { p.OwnerId, p.Signature }).IsUnique().HasDatabaseName("ux_skill_proposals_owner_signature");
    }
}

internal sealed class SyncDeviceConfiguration : IEntityTypeConfiguration<SyncDevice>
{
    public void Configure(EntityTypeBuilder<SyncDevice> builder)
    {
        builder.ToTable("sync_devices", MemorySchema.Name, t => t.HasCheckConstraint("ck_sync_devices_scopes", "cardinality(scopes) <= 16"));
        builder.HasKey(d => d.DeviceId);
        builder.Property(d => d.Scopes).IsRequired();
        builder.HasOne<Device>().WithMany().HasForeignKey(d => d.DeviceId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(d => d.OwnerId).HasDatabaseName("ix_sync_devices_owner");
    }
}

internal sealed class SyncReceiptConfiguration : IEntityTypeConfiguration<SyncReceipt>
{
    public void Configure(EntityTypeBuilder<SyncReceipt> builder)
    {
        builder.ToTable("sync_receipts", MemorySchema.Name);
        builder.HasKey(r => new { r.OwnerId, r.ChangeId });
        builder.Property(r => r.Response).HasColumnType("jsonb").IsRequired();
        builder.HasIndex(r => r.CreatedAt).HasDatabaseName("ix_sync_receipts_created");
    }
}

internal sealed class SyncConflictConfiguration : IEntityTypeConfiguration<SyncConflict>
{
    public void Configure(EntityTypeBuilder<SyncConflict> builder)
    {
        builder.ToTable("sync_conflicts", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_sync_conflicts_status", "status in ('open','resolved')");
            t.HasCheckConstraint("ck_sync_conflicts_kind", "kind in ('update_update','update_delete','delete_update','create_create')");
        });
        builder.HasKey(c => c.Id);
        builder.Property(c => c.Id).ValueGeneratedNever();
        builder.Property(c => c.ServerData).HasColumnType("jsonb");
        builder.Property(c => c.DeviceData).HasColumnType("jsonb");
        builder.HasOne<User>().WithMany().HasForeignKey(c => c.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(c => new { c.OwnerId, c.Status }).HasDatabaseName("ix_sync_conflicts_owner_status");
        builder.HasIndex(c => new { c.Entity, c.EntityId }).HasDatabaseName("ix_sync_conflicts_entity");
    }
}

internal sealed class JobConfiguration : IEntityTypeConfiguration<Job>
{
    public void Configure(EntityTypeBuilder<Job> builder)
    {
        builder.ToTable("jobs", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_jobs_type", "type ~ '^[a-z]+(\\.[a-z_]+)+$'");
            t.HasCheckConstraint("ck_jobs_name", "length(name) between 1 and 100");
            t.HasCheckConstraint("ck_jobs_schedule", $"schedule_kind in ({MemorySchema.InList(ScheduleKinds.All)})");
            t.HasCheckConstraint("ck_jobs_interval", "schedule_kind <> 'interval' or interval_seconds >= 60");
            t.HasCheckConstraint("ck_jobs_daily", "schedule_kind <> 'daily' or daily_minute_utc between 0 and 1439");
            t.HasCheckConstraint("ck_jobs_mode", "mode in ('read_only','mutating')");
            t.HasCheckConstraint("ck_jobs_approval", "approval_state in ('not_required','pending','approved','rejected')");
            t.HasCheckConstraint("ck_jobs_parameters", "jsonb_typeof(parameters) = 'object' and pg_column_size(parameters) <= 8192");
            t.HasCheckConstraint("ck_jobs_timeout", "timeout_seconds between 1 and 3600");
            t.HasCheckConstraint("ck_jobs_attempts", "max_attempts between 1 and 10");
        });
        builder.HasKey(j => j.Id);
        builder.Property(j => j.Id).ValueGeneratedNever();
        builder.Property(j => j.Parameters).HasColumnType("jsonb").IsRequired();
        builder.Ignore(j => j.CanRun);
        builder.HasOne<User>().WithMany().HasForeignKey(j => j.OwnerId).OnDelete(DeleteBehavior.Restrict);
        builder.HasIndex(j => j.NextRunAt).HasDatabaseName("ix_jobs_due").HasFilter("enabled and next_run_at is not null");
    }
}

internal sealed class JobRunConfiguration : IEntityTypeConfiguration<JobRun>
{
    public void Configure(EntityTypeBuilder<JobRun> builder)
    {
        builder.ToTable("job_runs", MemorySchema.Name, t =>
        {
            t.HasCheckConstraint("ck_job_runs_status", "status in ('queued','running','succeeded','failed','timed_out','cancelled')");
            t.HasCheckConstraint("ck_job_runs_triggered_by", "triggered_by in ('schedule','manual')");
            t.HasCheckConstraint("ck_job_runs_result", "result is null or pg_column_size(result) <= 32768");
            t.HasCheckConstraint("ck_job_runs_evidence", "evidence is null or pg_column_size(evidence) <= 32768");
        });
        builder.HasKey(r => r.Id);
        builder.Property(r => r.Id).ValueGeneratedNever();
        builder.Property(r => r.Result).HasColumnType("jsonb");
        builder.Property(r => r.Evidence).HasColumnType("jsonb");
        builder.HasOne<Job>().WithMany().HasForeignKey(r => r.JobId).OnDelete(DeleteBehavior.Cascade);
        builder.HasIndex(r => r.JobId).IsUnique().HasDatabaseName("ux_job_runs_one_active").HasFilter("status in ('queued','running')");
        builder.HasIndex(r => new { r.Status, r.ScheduledFor }).HasDatabaseName("ix_job_runs_claim").HasFilter("status in ('queued','running')");
        builder.HasIndex(r => new { r.JobId, r.CreatedAt }).HasDatabaseName("ix_job_runs_job_created");
    }
}
