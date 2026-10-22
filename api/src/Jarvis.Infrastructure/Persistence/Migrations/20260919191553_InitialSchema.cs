using System;
using System.Collections.Generic;
using Microsoft.EntityFrameworkCore.Migrations;
using Npgsql.EntityFrameworkCore.PostgreSQL.Metadata;

#nullable disable

namespace Jarvis.Infrastructure.Persistence.Migrations
{
    /// <inheritdoc />
    public partial class InitialSchema : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.EnsureSchema(
                name: "jarvis");

            migrationBuilder.AlterDatabase()
                .Annotation("Npgsql:PostgresExtension:citext", ",,");

            migrationBuilder.CreateTable(
                name: "users",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    email = table.Column<string>(type: "citext", nullable: false),
                    display_name = table.Column<string>(type: "text", nullable: false),
                    password_hash = table.Column<string>(type: "text", nullable: false),
                    status = table.Column<string>(type: "text", nullable: false),
                    failed_login_count = table.Column<int>(type: "integer", nullable: false),
                    locked_until = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_login_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_users", x => x.id);
                    table.CheckConstraint("ck_users_display_name_length", "length(display_name) between 1 and 100");
                    table.CheckConstraint("ck_users_email_length", "length(email) <= 254");
                    table.CheckConstraint("ck_users_status", "status in ('Active','Disabled')");
                });

            migrationBuilder.CreateTable(
                name: "devices",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    name = table.Column<string>(type: "text", nullable: false),
                    type = table.Column<string>(type: "text", nullable: false),
                    admin_state = table.Column<string>(type: "text", nullable: false),
                    presence_state = table.Column<string>(type: "text", nullable: false),
                    reported_busy = table.Column<bool>(type: "boolean", nullable: false),
                    current_session_id = table.Column<string>(type: "text", nullable: true),
                    capabilities = table.Column<List<string>>(type: "text[]", nullable: false),
                    supported_task_types = table.Column<List<string>>(type: "text[]", nullable: false),
                    reported_info = table.Column<string>(type: "jsonb", nullable: false),
                    last_seen_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_connected_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_disconnected_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    broker_synced_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    disabled_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    revoked_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    updated_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_devices", x => x.id);
                    table.CheckConstraint("ck_devices_admin_state", "admin_state in ('Active','Disabled','Revoked')");
                    table.CheckConstraint("ck_devices_capabilities", "cardinality(capabilities) <= 32");
                    table.CheckConstraint("ck_devices_name_length", "length(name) between 1 and 64");
                    table.CheckConstraint("ck_devices_presence_state", "presence_state in ('Online','Stale','Offline')");
                    table.CheckConstraint("ck_devices_reported_info", "jsonb_typeof(reported_info) = 'object'");
                    table.CheckConstraint("ck_devices_revoked_at", "admin_state <> 'Revoked' or revoked_at is not null");
                    table.CheckConstraint("ck_devices_session_id_length", "current_session_id is null or length(current_session_id) <= 64");
                    table.CheckConstraint("ck_devices_supported_task_types", "cardinality(supported_task_types) <= 64");
                    table.CheckConstraint("ck_devices_type", "type in ('Windows','ESP32','Mobile','Web','Unknown')");
                    table.ForeignKey(
                        name: "fk_devices_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "events",
                schema: "jarvis",
                columns: table => new
                {
                    seq = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityAlwaysColumn),
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: true),
                    type = table.Column<string>(type: "text", nullable: false),
                    occurred_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    recorded_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false, defaultValueSql: "now()"),
                    correlation_id = table.Column<Guid>(type: "uuid", nullable: false),
                    actor_type = table.Column<string>(type: "text", nullable: false),
                    actor_id = table.Column<Guid>(type: "uuid", nullable: true),
                    device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    task_id = table.Column<Guid>(type: "uuid", nullable: true),
                    data = table.Column<string>(type: "jsonb", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_events", x => x.seq);
                    table.CheckConstraint("ck_events_actor_type", "actor_type in ('User','Device','System')");
                    table.CheckConstraint("ck_events_data_size", "pg_column_size(data) <= 4096");
                    table.CheckConstraint("ck_events_type", "type ~ '^[A-Z][A-Z0-9_]{2,63}$'");
                    table.ForeignKey(
                        name: "fk_events_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "refresh_tokens",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    user_id = table.Column<Guid>(type: "uuid", nullable: false),
                    family_id = table.Column<Guid>(type: "uuid", nullable: false),
                    token_hash = table.Column<byte[]>(type: "bytea", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    expires_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    revoked_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    replaced_by_id = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_refresh_tokens", x => x.id);
                    table.ForeignKey(
                        name: "fk_refresh_tokens_replaced_by_id",
                        column: x => x.replaced_by_id,
                        principalSchema: "jarvis",
                        principalTable: "refresh_tokens",
                        principalColumn: "id",
                        onDelete: ReferentialAction.SetNull);
                    table.ForeignKey(
                        name: "fk_refresh_tokens_user_id",
                        column: x => x.user_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "device_credentials",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    kind = table.Column<string>(type: "text", nullable: false),
                    secret_hash = table.Column<byte[]>(type: "bytea", nullable: true),
                    secret_hint = table.Column<string>(type: "text", nullable: true),
                    certificate_thumbprint = table.Column<string>(type: "text", nullable: true),
                    status = table.Column<string>(type: "text", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    expires_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    revoked_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    revoked_reason = table.Column<string>(type: "text", nullable: true),
                    last_used_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_device_credentials", x => x.id);
                    table.CheckConstraint("ck_device_credentials_kind", "kind in ('Secret','ClientCertificate')");
                    table.CheckConstraint("ck_device_credentials_material", "(kind = 'Secret' and secret_hash is not null) or (kind = 'ClientCertificate' and certificate_thumbprint is not null)");
                    table.CheckConstraint("ck_device_credentials_revoked_reason", "revoked_reason is null or length(revoked_reason) <= 200");
                    table.CheckConstraint("ck_device_credentials_status", "status in ('Active','Revoked')");
                    table.ForeignKey(
                        name: "fk_device_credentials_device_id",
                        column: x => x.device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateTable(
                name: "device_task_grants",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    source_device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    target_device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    task_type = table.Column<string>(type: "text", nullable: false),
                    created_by_user_id = table.Column<Guid>(type: "uuid", nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_device_task_grants", x => x.id);
                    table.CheckConstraint("ck_grants_distinct_devices", "source_device_id <> target_device_id");
                    table.CheckConstraint("ck_grants_task_type", "task_type ~ '^[A-Z][A-Z0-9_]{1,63}$'");
                    table.ForeignKey(
                        name: "fk_device_task_grants_created_by_user_id",
                        column: x => x.created_by_user_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_device_task_grants_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_device_task_grants_source_device_id",
                        column: x => x.source_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                    table.ForeignKey(
                        name: "fk_device_task_grants_target_device_id",
                        column: x => x.target_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "tasks",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    requested_by_type = table.Column<string>(type: "text", nullable: false),
                    requested_by_user_id = table.Column<Guid>(type: "uuid", nullable: true),
                    source_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    target_device_id = table.Column<Guid>(type: "uuid", nullable: false),
                    type = table.Column<string>(type: "text", nullable: false),
                    payload = table.Column<string>(type: "jsonb", nullable: false),
                    status = table.Column<string>(type: "text", nullable: false),
                    status_reason = table.Column<string>(type: "text", nullable: true),
                    correlation_id = table.Column<Guid>(type: "uuid", nullable: false),
                    idempotency_key = table.Column<string>(type: "text", nullable: true),
                    request_hash = table.Column<byte[]>(type: "bytea", nullable: true),
                    attempt_count = table.Column<int>(type: "integer", nullable: false),
                    max_attempts = table.Column<int>(type: "integer", nullable: false),
                    expires_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    max_duration_seconds = table.Column<int>(type: "integer", nullable: true),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    dispatched_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    delivered_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    started_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    completed_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    cancel_requested_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    last_update_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: true),
                    progress_percent = table.Column<short>(type: "smallint", nullable: true),
                    progress_message = table.Column<string>(type: "text", nullable: true),
                    result = table.Column<string>(type: "jsonb", nullable: true),
                    error_code = table.Column<string>(type: "text", nullable: true),
                    error_message = table.Column<string>(type: "text", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_tasks", x => x.id);
                    table.CheckConstraint("ck_tasks_completed_result", "status <> 'Completed' or result is not null");
                    table.CheckConstraint("ck_tasks_error_code", "error_code is null or length(error_code) <= 64");
                    table.CheckConstraint("ck_tasks_error_message", "error_message is null or length(error_message) <= 500");
                    table.CheckConstraint("ck_tasks_failed_error", "status not in ('Failed','Cancelled','Expired') or error_code is not null");
                    table.CheckConstraint("ck_tasks_idempotency_key", "idempotency_key is null or length(idempotency_key) between 1 and 100");
                    table.CheckConstraint("ck_tasks_max_duration", "max_duration_seconds is null or max_duration_seconds between 1 and 86400");
                    table.CheckConstraint("ck_tasks_payload_object", "jsonb_typeof(payload) = 'object'");
                    table.CheckConstraint("ck_tasks_payload_size", "pg_column_size(payload) <= 16384");
                    table.CheckConstraint("ck_tasks_progress_message", "progress_message is null or length(progress_message) <= 200");
                    table.CheckConstraint("ck_tasks_progress_percent", "progress_percent is null or progress_percent between 0 and 100");
                    table.CheckConstraint("ck_tasks_requested_by_type", "requested_by_type in ('User','Device')");
                    table.CheckConstraint("ck_tasks_requester", "(requested_by_type = 'User' and requested_by_user_id is not null and source_device_id is null) or (requested_by_type = 'Device' and source_device_id is not null and requested_by_user_id is null)");
                    table.CheckConstraint("ck_tasks_result_size", "result is null or pg_column_size(result) <= 32768");
                    table.CheckConstraint("ck_tasks_status", "status in ('Pending','Dispatched','Delivered','Running','WaitingForApproval','Completed','Failed','Cancelled','Expired')");
                    table.CheckConstraint("ck_tasks_status_reason", "status_reason is null or length(status_reason) <= 100");
                    table.CheckConstraint("ck_tasks_terminal_completed_at", "status not in ('Completed','Failed','Cancelled','Expired') or completed_at is not null");
                    table.CheckConstraint("ck_tasks_type", "type ~ '^[A-Z][A-Z0-9_]{1,63}$'");
                    table.ForeignKey(
                        name: "fk_tasks_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_tasks_requested_by_user_id",
                        column: x => x.requested_by_user_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_tasks_source_device_id",
                        column: x => x.source_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_tasks_target_device_id",
                        column: x => x.target_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                });

            migrationBuilder.CreateIndex(
                name: "ux_device_credentials_active",
                schema: "jarvis",
                table: "device_credentials",
                column: "device_id",
                unique: true,
                filter: "status = 'Active'");

            migrationBuilder.CreateIndex(
                name: "ix_device_task_grants_created_by_user_id",
                schema: "jarvis",
                table: "device_task_grants",
                column: "created_by_user_id");

            migrationBuilder.CreateIndex(
                name: "ix_device_task_grants_owner_id",
                schema: "jarvis",
                table: "device_task_grants",
                column: "owner_id");

            migrationBuilder.CreateIndex(
                name: "ix_grants_target_device_id",
                schema: "jarvis",
                table: "device_task_grants",
                column: "target_device_id");

            migrationBuilder.CreateIndex(
                name: "ux_grants",
                schema: "jarvis",
                table: "device_task_grants",
                columns: new[] { "source_device_id", "target_device_id", "task_type" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ix_devices_broker_unsynced",
                schema: "jarvis",
                table: "devices",
                column: "id",
                filter: "broker_synced_at is null");

            migrationBuilder.CreateIndex(
                name: "ix_devices_presence_sweep",
                schema: "jarvis",
                table: "devices",
                columns: new[] { "presence_state", "last_seen_at" },
                filter: "presence_state <> 'Offline'");

            migrationBuilder.CreateIndex(
                name: "ux_devices_owner_name",
                schema: "jarvis",
                table: "devices",
                columns: new[] { "owner_id", "name" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ix_events_correlation",
                schema: "jarvis",
                table: "events",
                column: "correlation_id");

            migrationBuilder.CreateIndex(
                name: "ix_events_device",
                schema: "jarvis",
                table: "events",
                columns: new[] { "device_id", "seq" },
                filter: "device_id is not null");

            migrationBuilder.CreateIndex(
                name: "ix_events_owner_seq",
                schema: "jarvis",
                table: "events",
                columns: new[] { "owner_id", "seq" });

            migrationBuilder.CreateIndex(
                name: "ix_events_recorded_at",
                schema: "jarvis",
                table: "events",
                column: "recorded_at");

            migrationBuilder.CreateIndex(
                name: "ix_events_task",
                schema: "jarvis",
                table: "events",
                columns: new[] { "task_id", "seq" },
                filter: "task_id is not null");

            migrationBuilder.CreateIndex(
                name: "ux_events_id",
                schema: "jarvis",
                table: "events",
                column: "id",
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ix_refresh_tokens_family_id",
                schema: "jarvis",
                table: "refresh_tokens",
                column: "family_id");

            migrationBuilder.CreateIndex(
                name: "ix_refresh_tokens_replaced_by_id",
                schema: "jarvis",
                table: "refresh_tokens",
                column: "replaced_by_id");

            migrationBuilder.CreateIndex(
                name: "ix_refresh_tokens_user_id",
                schema: "jarvis",
                table: "refresh_tokens",
                column: "user_id");

            migrationBuilder.CreateIndex(
                name: "ux_refresh_tokens_token_hash",
                schema: "jarvis",
                table: "refresh_tokens",
                column: "token_hash",
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ix_tasks_ack_timeout",
                schema: "jarvis",
                table: "tasks",
                column: "dispatched_at",
                filter: "status = 'Dispatched'");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_active_by_target",
                schema: "jarvis",
                table: "tasks",
                column: "target_device_id",
                filter: "status in ('Dispatched','Delivered','Running','WaitingForApproval')");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_correlation",
                schema: "jarvis",
                table: "tasks",
                column: "correlation_id");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_dispatch",
                schema: "jarvis",
                table: "tasks",
                columns: new[] { "target_device_id", "created_at" },
                filter: "status = 'Pending'");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_expiry",
                schema: "jarvis",
                table: "tasks",
                column: "expires_at",
                filter: "status in ('Pending','Dispatched')");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_owner_created",
                schema: "jarvis",
                table: "tasks",
                columns: new[] { "owner_id", "created_at", "id" },
                descending: new[] { false, true, true });

            migrationBuilder.CreateIndex(
                name: "ix_tasks_requested_by_user_id",
                schema: "jarvis",
                table: "tasks",
                column: "requested_by_user_id");

            migrationBuilder.CreateIndex(
                name: "ix_tasks_source_created",
                schema: "jarvis",
                table: "tasks",
                columns: new[] { "source_device_id", "created_at" },
                descending: new[] { false, true },
                filter: "source_device_id is not null");

            migrationBuilder.CreateIndex(
                name: "ux_tasks_idempotency",
                schema: "jarvis",
                table: "tasks",
                columns: new[] { "owner_id", "requested_by_type", "requested_by_user_id", "source_device_id", "idempotency_key" },
                unique: true,
                filter: "idempotency_key is not null")
                .Annotation("Npgsql:NullsDistinct", false);

            migrationBuilder.CreateIndex(
                name: "ux_users_email",
                schema: "jarvis",
                table: "users",
                column: "email",
                unique: true);

            // Mínimo privilegio (docs/04-data/database-design.md#roles-de-base-de-datos).
            // Solo aplica si los roles existen (producción); en desarrollo se omite.
            migrationBuilder.Sql("""
                DO $$
                BEGIN
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_app') THEN
                    GRANT USAGE ON SCHEMA jarvis TO jarvis_app;
                    GRANT SELECT, INSERT, UPDATE ON jarvis.users, jarvis.devices, jarvis.device_credentials, jarvis.tasks TO jarvis_app;
                    GRANT SELECT, INSERT, UPDATE, DELETE ON jarvis.refresh_tokens, jarvis.device_task_grants TO jarvis_app;
                    GRANT SELECT, INSERT ON jarvis.events TO jarvis_app;
                    GRANT SELECT ON jarvis.__ef_migrations_history TO jarvis_app;
                    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA jarvis TO jarvis_app;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_maintenance') THEN
                    GRANT USAGE ON SCHEMA jarvis TO jarvis_maintenance;
                    GRANT SELECT, DELETE ON jarvis.events, jarvis.refresh_tokens TO jarvis_maintenance;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_backup') THEN
                    GRANT USAGE ON SCHEMA jarvis TO jarvis_backup;
                    GRANT SELECT ON ALL TABLES IN SCHEMA jarvis TO jarvis_backup;
                  END IF;
                END
                $$;
                """);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "device_credentials",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "device_task_grants",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "events",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "refresh_tokens",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "tasks",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "devices",
                schema: "jarvis");

            migrationBuilder.DropTable(
                name: "users",
                schema: "jarvis");
        }
    }
}
