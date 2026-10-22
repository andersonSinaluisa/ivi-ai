using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Jarvis.Infrastructure.Persistence.Migrations
{
    /// <inheritdoc />
    public partial class MemorySyncAndJobs : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.EnsureSchema(
                name: "memory");

            migrationBuilder.CreateSequence(
                name: "change_seq",
                schema: "memory");

            migrationBuilder.CreateTable(
                name: "experiences",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    goal = table.Column<string>(type: "text", nullable: false),
                    context = table.Column<string>(type: "text", nullable: false),
                    plan = table.Column<string>(type: "text", nullable: false),
                    steps = table.Column<string>(type: "jsonb", nullable: false),
                    tools = table.Column<string[]>(type: "text[]", nullable: false),
                    result = table.Column<string>(type: "text", nullable: false),
                    evidence = table.Column<string>(type: "jsonb", nullable: false),
                    errors = table.Column<string>(type: "jsonb", nullable: false),
                    procedure = table.Column<string>(type: "text", nullable: true),
                    verified = table.Column<bool>(type: "boolean", nullable: false),
                    completed = table.Column<bool>(type: "boolean", nullable: false),
                    occurrences = table.Column<int>(type: "integer", nullable: false),
                    signature = table.Column<string>(type: "text", nullable: false),
                    origin_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    last_change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    version = table.Column<int>(type: "integer", nullable: false),
                    seq = table.Column<long>(type: "bigint", nullable: false),
                    device_created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    deleted_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_experiences", x => x.id);
                    table.CheckConstraint("ck_experiences_goal", "deleted_at is not null or length(goal) between 1 and 2000");
                    table.CheckConstraint("ck_experiences_occurrences", "occurrences >= 1");
                    table.CheckConstraint("ck_experiences_size", "pg_column_size(steps) + pg_column_size(evidence) + pg_column_size(errors) <= 65536");
                    table.CheckConstraint("ck_experiences_steps", "jsonb_typeof(steps) = 'array'");
                    table.ForeignKey(
                        name: "fk_experiences_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "jobs",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    type = table.Column<string>(type: "text", nullable: false),
                    name = table.Column<string>(type: "text", nullable: false),
                    schedule_kind = table.Column<string>(type: "text", nullable: false),
                    interval_seconds = table.Column<int>(type: "integer", nullable: true),
                    daily_minute_utc = table.Column<int>(type: "integer", nullable: true),
                    next_run_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    enabled = table.Column<bool>(type: "boolean", nullable: false),
                    mode = table.Column<string>(type: "text", nullable: false),
                    approval_state = table.Column<string>(type: "text", nullable: false),
                    parameters = table.Column<string>(type: "jsonb", nullable: false),
                    timeout_seconds = table.Column<int>(type: "integer", nullable: false),
                    max_attempts = table.Column<int>(type: "integer", nullable: false),
                    created_by_id = table.Column<Guid>(type: "uuid", nullable: false),
                    approved_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    approved_by_id = table.Column<Guid>(type: "uuid", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_jobs", x => x.id);
                    table.CheckConstraint("ck_jobs_approval", "approval_state in ('not_required','pending','approved','rejected')");
                    table.CheckConstraint("ck_jobs_attempts", "max_attempts between 1 and 10");
                    table.CheckConstraint("ck_jobs_daily", "schedule_kind <> 'daily' or daily_minute_utc between 0 and 1439");
                    table.CheckConstraint("ck_jobs_interval", "schedule_kind <> 'interval' or interval_seconds >= 60");
                    table.CheckConstraint("ck_jobs_mode", "mode in ('read_only','mutating')");
                    table.CheckConstraint("ck_jobs_name", "length(name) between 1 and 100");
                    table.CheckConstraint("ck_jobs_parameters", "jsonb_typeof(parameters) = 'object' and pg_column_size(parameters) <= 8192");
                    table.CheckConstraint("ck_jobs_schedule", "schedule_kind in ('once','interval','daily')");
                    table.CheckConstraint("ck_jobs_timeout", "timeout_seconds between 1 and 3600");
                    table.CheckConstraint("ck_jobs_type", "type ~ '^[a-z]+(\\.[a-z_]+)+$'");
                    table.ForeignKey(
                        name: "fk_jobs_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "memories",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    kind = table.Column<string>(type: "text", nullable: false),
                    content = table.Column<string>(type: "text", nullable: true),
                    summary = table.Column<string>(type: "text", nullable: false),
                    tags = table.Column<string[]>(type: "text[]", nullable: false),
                    relevance = table.Column<float>(type: "real", nullable: false),
                    confidence = table.Column<float>(type: "real", nullable: false),
                    verification = table.Column<string>(type: "text", nullable: false),
                    retention_kind = table.Column<string>(type: "text", nullable: false),
                    retention_days = table.Column<int>(type: "integer", nullable: true),
                    source_kind = table.Column<string>(type: "text", nullable: false),
                    source_reference = table.Column<string>(type: "text", nullable: true),
                    source_label = table.Column<string>(type: "text", nullable: false),
                    content_sha256 = table.Column<byte[]>(type: "bytea", nullable: true),
                    origin_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    last_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    last_change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    version = table.Column<int>(type: "integer", nullable: false),
                    seq = table.Column<long>(type: "bigint", nullable: false),
                    has_open_conflict = table.Column<bool>(type: "boolean", nullable: false),
                    device_created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    device_updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    deleted_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_memories", x => x.id);
                    table.CheckConstraint("ck_memories_content", "deleted_at is not null or (content is not null and length(content) between 1 and 8000)");
                    table.CheckConstraint("ck_memories_kind", "kind in ('episodic','semantic','procedural','experience','entity')");
                    table.CheckConstraint("ck_memories_retention", "retention_kind in ('permanent','days') and (retention_kind <> 'days' or retention_days between 1 and 36500)");
                    table.CheckConstraint("ck_memories_scores", "relevance between 0 and 1 and confidence between 0 and 1");
                    table.CheckConstraint("ck_memories_summary", "length(summary) <= 500");
                    table.CheckConstraint("ck_memories_tags", "cardinality(tags) <= 32");
                    table.CheckConstraint("ck_memories_verification", "verification in ('unverified','verified','disputed')");
                    table.CheckConstraint("ck_memories_version", "version >= 1");
                    table.ForeignKey(
                        name: "fk_memories_origin_device_id",
                        column: x => x.origin_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_memories_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "skill_proposals",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    signature = table.Column<string>(type: "text", nullable: false),
                    title = table.Column<string>(type: "text", nullable: false),
                    procedure = table.Column<string>(type: "text", nullable: false),
                    experience_ids = table.Column<Guid[]>(type: "uuid[]", nullable: false),
                    support_count = table.Column<int>(type: "integer", nullable: false),
                    status = table.Column<string>(type: "text", nullable: false),
                    skill_id = table.Column<Guid>(type: "uuid", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    decided_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_skill_proposals", x => x.id);
                    table.CheckConstraint("ck_skill_proposals_status", "status in ('pending','accepted','rejected')");
                    table.ForeignKey(
                        name: "fk_skill_proposals_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "skills",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    name = table.Column<string>(type: "text", nullable: false),
                    description = table.Column<string>(type: "text", nullable: false),
                    procedure = table.Column<string>(type: "text", nullable: false),
                    procedure_version = table.Column<int>(type: "integer", nullable: false),
                    status = table.Column<string>(type: "text", nullable: false),
                    compatibility = table.Column<string>(type: "jsonb", nullable: false),
                    permissions = table.Column<string>(type: "jsonb", nullable: false),
                    source = table.Column<string>(type: "text", nullable: false),
                    experience_id = table.Column<Guid>(type: "uuid", nullable: true),
                    verified_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    verified_by = table.Column<Guid>(type: "uuid", nullable: true),
                    origin_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    last_change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    version = table.Column<int>(type: "integer", nullable: false),
                    seq = table.Column<long>(type: "bigint", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    deleted_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_skills", x => x.id);
                    table.CheckConstraint("ck_skills_name", "deleted_at is not null or length(name) between 1 and 200");
                    table.CheckConstraint("ck_skills_procedure", "length(procedure) <= 20000");
                    table.CheckConstraint("ck_skills_status", "status in ('draft','verified','deprecated')");
                    table.CheckConstraint("ck_skills_verified", "status <> 'verified' or verified_at is not null");
                    table.ForeignKey(
                        name: "fk_skills_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "sync_conflicts",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    entity = table.Column<string>(type: "text", nullable: false),
                    entity_id = table.Column<Guid>(type: "uuid", nullable: false),
                    kind = table.Column<string>(type: "text", nullable: false),
                    fields = table.Column<string[]>(type: "text[]", nullable: false),
                    base_version = table.Column<int>(type: "integer", nullable: true),
                    server_version = table.Column<int>(type: "integer", nullable: false),
                    server_data = table.Column<string>(type: "jsonb", nullable: true),
                    device_data = table.Column<string>(type: "jsonb", nullable: true),
                    device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    status = table.Column<string>(type: "text", nullable: false),
                    resolution = table.Column<string>(type: "text", nullable: true),
                    resolved_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    resolved_by_type = table.Column<string>(type: "text", nullable: true),
                    resolved_by_id = table.Column<Guid>(type: "uuid", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_sync_conflicts", x => x.id);
                    table.CheckConstraint("ck_sync_conflicts_kind", "kind in ('update_update','update_delete','delete_update','create_create')");
                    table.CheckConstraint("ck_sync_conflicts_status", "status in ('open','resolved')");
                    table.ForeignKey(
                        name: "fk_sync_conflicts_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "sync_devices",
                schema: "memory",
                columns: table => new
                {
                    device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    scopes = table.Column<string[]>(type: "text[]", nullable: false),
                    last_push_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_pull_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_pull_seq = table.Column<long>(type: "bigint", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_sync_devices", x => x.device_id);
                    table.CheckConstraint("ck_sync_devices_scopes", "cardinality(scopes) <= 16");
                    table.ForeignKey(
                        name: "fk_sync_devices_device_id",
                        column: x => x.device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "sync_receipts",
                schema: "memory",
                columns: table => new
                {
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    change_id = table.Column<Guid>(type: "uuid", nullable: false),
                    device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    response = table.Column<string>(type: "jsonb", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_sync_receipts", x => new { x.owner_id, x.change_id });
                });

            migrationBuilder.CreateTable(
                name: "job_runs",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    job_id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    attempt = table.Column<int>(type: "integer", nullable: false),
                    status = table.Column<string>(type: "text", nullable: false),
                    triggered_by = table.Column<string>(type: "text", nullable: false),
                    scheduled_for = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    lease_until = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    worker_id = table.Column<string>(type: "text", nullable: true),
                    started_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    finished_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    result = table.Column<string>(type: "jsonb", nullable: true),
                    evidence = table.Column<string>(type: "jsonb", nullable: true),
                    error = table.Column<string>(type: "text", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_job_runs", x => x.id);
                    table.CheckConstraint("ck_job_runs_evidence", "evidence is null or pg_column_size(evidence) <= 32768");
                    table.CheckConstraint("ck_job_runs_result", "result is null or pg_column_size(result) <= 32768");
                    table.CheckConstraint("ck_job_runs_status", "status in ('queued','running','succeeded','failed','timed_out','cancelled')");
                    table.CheckConstraint("ck_job_runs_triggered_by", "triggered_by in ('schedule','manual')");
                    table.ForeignKey(
                        name: "fk_job_runs_job_id",
                        column: x => x.job_id,
                        principalSchema: "memory",
                        principalTable: "jobs",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "memory_embeddings",
                schema: "memory",
                columns: table => new
                {
                    memory_id = table.Column<Guid>(type: "uuid", nullable: false),
                    model = table.Column<string>(type: "text", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    dims = table.Column<int>(type: "integer", nullable: false),
                    content_sha256 = table.Column<byte[]>(type: "bytea", nullable: false),
                    vector = table.Column<float[]>(type: "real[]", nullable: false),
                    last_change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    origin_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    version = table.Column<int>(type: "integer", nullable: false),
                    seq = table.Column<long>(type: "bigint", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_memory_embeddings", x => new { x.memory_id, x.model });
                    table.CheckConstraint("ck_memory_embeddings_dims", "dims between 1 and 4096 and cardinality(vector) = dims");
                    table.CheckConstraint("ck_memory_embeddings_model", "length(model) between 1 and 128");
                    table.ForeignKey(
                        name: "fk_memory_embeddings_memory_id",
                        column: x => x.memory_id,
                        principalSchema: "memory",
                        principalTable: "memories",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "memory_relations",
                schema: "memory",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    source_id = table.Column<Guid>(type: "uuid", nullable: false),
                    target_id = table.Column<Guid>(type: "uuid", nullable: false),
                    kind = table.Column<string>(type: "text", nullable: false),
                    confidence = table.Column<float>(type: "real", nullable: false),
                    provenance = table.Column<string>(type: "text", nullable: false),
                    verified = table.Column<bool>(type: "boolean", nullable: false),
                    note = table.Column<string>(type: "text", nullable: true),
                    origin_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    last_change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    version = table.Column<int>(type: "integer", nullable: false),
                    seq = table.Column<long>(type: "bigint", nullable: false),
                    device_created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    deleted_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_memory_relations", x => x.id);
                    table.CheckConstraint("ck_memory_relations_confidence", "confidence between 0 and 1");
                    table.CheckConstraint("ck_memory_relations_distinct", "source_id <> target_id");
                    table.CheckConstraint("ck_memory_relations_inferred_unverified", "provenance <> 'inferred' or not verified");
                    table.CheckConstraint("ck_memory_relations_kind", "kind in ('BELONGS_TO','DEPENDS_ON','CAUSED_BY','SOLVED_BY','RELATED_TO','LEARNED_FROM','USED_IN','DUPLICATE_OF')");
                    table.CheckConstraint("ck_memory_relations_note", "note is null or length(note) <= 500");
                    table.CheckConstraint("ck_memory_relations_provenance", "provenance in ('user','agent','inferred','import','consolidation')");
                    table.ForeignKey(
                        name: "fk_memory_relations_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_memory_relations_source_id",
                        column: x => x.source_id,
                        principalSchema: "memory",
                        principalTable: "memories",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_memory_relations_target_id",
                        column: x => x.target_id,
                        principalSchema: "memory",
                        principalTable: "memories",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "memory_versions",
                schema: "memory",
                columns: table => new
                {
                    memory_id = table.Column<Guid>(type: "uuid", nullable: false),
                    version = table.Column<int>(type: "integer", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    data = table.Column<string>(type: "jsonb", nullable: false),
                    change_id = table.Column<Guid>(type: "uuid", nullable: true),
                    device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    recorded_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_memory_versions", x => new { x.memory_id, x.version });
                    table.CheckConstraint("ck_memory_versions_data", "jsonb_typeof(data) = 'object'");
                    table.ForeignKey(
                        name: "fk_memory_versions_memory_id",
                        column: x => x.memory_id,
                        principalSchema: "memory",
                        principalTable: "memories",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "skill_versions",
                schema: "memory",
                columns: table => new
                {
                    skill_id = table.Column<Guid>(type: "uuid", nullable: false),
                    number = table.Column<int>(type: "integer", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    procedure = table.Column<string>(type: "text", nullable: false),
                    changelog = table.Column<string>(type: "text", nullable: true),
                    created_by_type = table.Column<string>(type: "text", nullable: false),
                    created_by_id = table.Column<Guid>(type: "uuid", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_skill_versions", x => new { x.skill_id, x.number });
                    table.ForeignKey(
                        name: "fk_skill_versions_skill_id",
                        column: x => x.skill_id,
                        principalSchema: "memory",
                        principalTable: "skills",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateIndex(
                name: "ix_experiences_owner_seq",
                schema: "memory",
                table: "experiences",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_experiences_owner_signature",
                schema: "memory",
                table: "experiences",
                columns: new[] { "owner_id", "signature" },
                filter: "deleted_at is null");

            migrationBuilder.CreateIndex(
                name: "ix_job_runs_claim",
                schema: "memory",
                table: "job_runs",
                columns: new[] { "status", "scheduled_for" },
                filter: "status in ('queued','running')");

            migrationBuilder.CreateIndex(
                name: "ix_job_runs_job_created",
                schema: "memory",
                table: "job_runs",
                columns: new[] { "job_id", "created_at" });

            migrationBuilder.CreateIndex(
                name: "ux_job_runs_one_active",
                schema: "memory",
                table: "job_runs",
                column: "job_id",
                unique: true,
                filter: "status in ('queued','running')");

            migrationBuilder.CreateIndex(
                name: "ix_jobs_due",
                schema: "memory",
                table: "jobs",
                column: "next_run_at",
                filter: "enabled and next_run_at is not null");

            migrationBuilder.CreateIndex(
                name: "ix_jobs_owner_id",
                schema: "memory",
                table: "jobs",
                column: "owner_id");

            migrationBuilder.CreateIndex(
                name: "ix_memories_origin_device_id",
                schema: "memory",
                table: "memories",
                column: "origin_device_id");

            migrationBuilder.CreateIndex(
                name: "ix_memories_owner_seq",
                schema: "memory",
                table: "memories",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_memories_owner_updated",
                schema: "memory",
                table: "memories",
                columns: new[] { "owner_id", "updated_at" },
                filter: "deleted_at is null");

            migrationBuilder.CreateIndex(
                name: "ix_memory_embeddings_owner_model",
                schema: "memory",
                table: "memory_embeddings",
                columns: new[] { "owner_id", "model", "dims" });

            migrationBuilder.CreateIndex(
                name: "ix_memory_embeddings_owner_seq",
                schema: "memory",
                table: "memory_embeddings",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_memory_relations_owner_seq",
                schema: "memory",
                table: "memory_relations",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_memory_relations_source",
                schema: "memory",
                table: "memory_relations",
                column: "source_id",
                filter: "deleted_at is null");

            migrationBuilder.CreateIndex(
                name: "ix_memory_relations_target",
                schema: "memory",
                table: "memory_relations",
                column: "target_id",
                filter: "deleted_at is null");

            migrationBuilder.CreateIndex(
                name: "ix_memory_versions_owner",
                schema: "memory",
                table: "memory_versions",
                column: "owner_id");

            migrationBuilder.CreateIndex(
                name: "ux_skill_proposals_owner_signature",
                schema: "memory",
                table: "skill_proposals",
                columns: new[] { "owner_id", "signature" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ix_skills_owner_seq",
                schema: "memory",
                table: "skills",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_sync_conflicts_entity",
                schema: "memory",
                table: "sync_conflicts",
                columns: new[] { "entity", "entity_id" });

            migrationBuilder.CreateIndex(
                name: "ix_sync_conflicts_owner_status",
                schema: "memory",
                table: "sync_conflicts",
                columns: new[] { "owner_id", "status" });

            migrationBuilder.CreateIndex(
                name: "ix_sync_devices_owner",
                schema: "memory",
                table: "sync_devices",
                column: "owner_id");

            migrationBuilder.CreateIndex(
                name: "ix_sync_receipts_created",
                schema: "memory",
                table: "sync_receipts",
                column: "created_at");

            // Búsqueda de texto (configuración 'simple': sin raíces, sirve para español e inglés). La expresión debe
            // coincidir con MemorySql.SearchDocument / ExperienceDocument para que el índice se use.
            migrationBuilder.Sql("""
                CREATE INDEX ix_memories_search ON memory.memories
                  USING gin (to_tsvector('simple', summary || ' ' || coalesce(content, '')))
                  WHERE deleted_at IS NULL;
                CREATE INDEX ix_experiences_search ON memory.experiences
                  USING gin (to_tsvector('simple', goal || ' ' || result))
                  WHERE deleted_at IS NULL;
                """);

            // Mínimo privilegio (docs/04-data/database-design.md#roles-de-base-de-datos). Solo si los roles existen.
            migrationBuilder.Sql("""
                DO $$
                BEGIN
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_app') THEN
                    GRANT USAGE ON SCHEMA memory TO jarvis_app;
                    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA memory TO jarvis_app;
                    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA memory TO jarvis_app;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_worker') THEN
                    GRANT USAGE ON SCHEMA memory, jarvis TO jarvis_worker;
                    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA memory TO jarvis_worker;
                    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA memory TO jarvis_worker;
                    GRANT SELECT, INSERT ON jarvis.events TO jarvis_worker;
                    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA jarvis TO jarvis_worker;
                    GRANT SELECT ON jarvis.__ef_migrations_history TO jarvis_worker;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_maintenance') THEN
                    GRANT USAGE ON SCHEMA memory TO jarvis_maintenance;
                    GRANT SELECT, DELETE ON memory.sync_receipts TO jarvis_maintenance;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_backup') THEN
                    GRANT USAGE ON SCHEMA memory TO jarvis_backup;
                    GRANT SELECT ON ALL TABLES IN SCHEMA memory TO jarvis_backup;
                  END IF;
                END
                $$;
                """);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "experiences",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "job_runs",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "memory_embeddings",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "memory_relations",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "memory_versions",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "skill_proposals",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "skill_versions",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "sync_conflicts",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "sync_devices",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "sync_receipts",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "jobs",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "memories",
                schema: "memory");

            migrationBuilder.DropTable(
                name: "skills",
                schema: "memory");

            migrationBuilder.DropSequence(
                name: "change_seq",
                schema: "memory");
        }
    }
}
