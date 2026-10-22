using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Jarvis.Infrastructure.Persistence.Migrations
{
    /// <inheritdoc />
    public partial class TaskSignaturesAndApprovals : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<string>(
                name: "signature_alg",
                schema: "jarvis",
                table: "tasks",
                type: "character varying(16)",
                maxLength: 16,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "signature_key_id",
                schema: "jarvis",
                table: "tasks",
                type: "character varying(64)",
                maxLength: 64,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "signature_value",
                schema: "jarvis",
                table: "tasks",
                type: "character varying(128)",
                maxLength: 128,
                nullable: true);

            migrationBuilder.CreateTable(
                name: "task_approvals",
                schema: "jarvis",
                columns: table => new
                {
                    id = table.Column<Guid>(type: "uuid", nullable: false),
                    owner_id = table.Column<Guid>(type: "uuid", nullable: false),
                    task_id = table.Column<Guid>(type: "uuid", nullable: false),
                    decision = table.Column<string>(type: "text", nullable: false),
                    decided_by_type = table.Column<string>(type: "text", nullable: false),
                    decided_by_user_id = table.Column<Guid>(type: "uuid", nullable: true),
                    decided_by_device_id = table.Column<Guid>(type: "uuid", nullable: true),
                    decided_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false),
                    nonce = table.Column<string>(type: "character varying(128)", maxLength: 128, nullable: false),
                    signature_alg = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    signature_key_id = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    signature_value = table.Column<string>(type: "character varying(128)", maxLength: 128, nullable: false),
                    created_at = table.Column<DateTimeOffset>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("pk_task_approvals", x => x.id);
                    table.CheckConstraint("ck_task_approvals_decided_by_type", "decided_by_type in ('User','Device')");
                    table.CheckConstraint("ck_task_approvals_decider", "(decided_by_type = 'User' and decided_by_user_id is not null and decided_by_device_id is null) or (decided_by_type = 'Device' and decided_by_device_id is not null and decided_by_user_id is null)");
                    table.CheckConstraint("ck_task_approvals_decision", "decision in ('Granted','Denied')");
                    table.CheckConstraint("ck_task_approvals_signature", "signature_alg = 'Ed25519'");
                    table.ForeignKey(
                        name: "fk_task_approvals_decided_by_device_id",
                        column: x => x.decided_by_device_id,
                        principalSchema: "jarvis",
                        principalTable: "devices",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_task_approvals_decided_by_user_id",
                        column: x => x.decided_by_user_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_task_approvals_owner_id",
                        column: x => x.owner_id,
                        principalSchema: "jarvis",
                        principalTable: "users",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Restrict);
                    table.ForeignKey(
                        name: "fk_task_approvals_task_id",
                        column: x => x.task_id,
                        principalSchema: "jarvis",
                        principalTable: "tasks",
                        principalColumn: "id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.AddCheckConstraint(
                name: "ck_tasks_signature_complete",
                schema: "jarvis",
                table: "tasks",
                sql: "(signature_alg is null and signature_key_id is null and signature_value is null) or (signature_alg = 'Ed25519' and signature_key_id is not null and signature_value is not null)");

            migrationBuilder.CreateIndex(
                name: "ix_task_approvals_decided_by_device_id",
                schema: "jarvis",
                table: "task_approvals",
                column: "decided_by_device_id");

            migrationBuilder.CreateIndex(
                name: "ix_task_approvals_decided_by_user_id",
                schema: "jarvis",
                table: "task_approvals",
                column: "decided_by_user_id");

            migrationBuilder.CreateIndex(
                name: "ux_task_approvals_nonce",
                schema: "jarvis",
                table: "task_approvals",
                columns: new[] { "owner_id", "nonce" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "ux_task_approvals_task",
                schema: "jarvis",
                table: "task_approvals",
                column: "task_id",
                unique: true);

            // La tabla nueva necesita sus propios GRANT: los de InitialSchema no la alcanzan.
            // Solo aplica si los roles existen (producción); en desarrollo se omite.
            migrationBuilder.Sql("""
                DO $$
                BEGIN
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_app') THEN
                    GRANT SELECT, INSERT ON jarvis.task_approvals TO jarvis_app;
                  END IF;
                  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_backup') THEN
                    GRANT SELECT ON jarvis.task_approvals TO jarvis_backup;
                  END IF;
                END
                $$;
                """);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "task_approvals",
                schema: "jarvis");

            migrationBuilder.DropCheckConstraint(
                name: "ck_tasks_signature_complete",
                schema: "jarvis",
                table: "tasks");

            migrationBuilder.DropColumn(
                name: "signature_alg",
                schema: "jarvis",
                table: "tasks");

            migrationBuilder.DropColumn(
                name: "signature_key_id",
                schema: "jarvis",
                table: "tasks");

            migrationBuilder.DropColumn(
                name: "signature_value",
                schema: "jarvis",
                table: "tasks");
        }
    }
}
