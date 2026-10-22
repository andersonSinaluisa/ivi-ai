# User Stories

66 historias (`HU-WIN-001` … `HU-WIN-066`); 53 forman el MVP. Cada HU tiene: **ID, Title, Goal, Description, Acceptance Criteria, Dependencies, Technical Considerations, Tests, Definition of Done**, y enlaza la documentación que es su fuente de verdad.

Uso previsto:

```text
Implementa HU-WIN-014 utilizando docs/ como fuente de verdad.
```

Quien implemente una HU debe leer la HU, los documentos que enlaza y sus dependencias. Si la documentación y la HU se contradicen, prevalece la documentación de diseño y se corrige la HU en el mismo PR.

## Definition of Done común

Aplica a todas las HUs además de su DoD específica:

1. Criterios de aceptación cumplidos y demostrados con tests automatizados (o, si requieren escritorio interactivo, con un procedimiento reproducible documentado en el PR).
2. `cargo fmt`, `cargo clippy -D warnings` y `cargo test` verdes en CI (Linux para crates portables, Windows para el workspace).
3. Sin `unsafe` fuera de `jarvis-windows`; cada bloque `unsafe` con comentario `// SAFETY:`.
4. Ningún secreto, contenido de archivo, título, salida de comando ni payload sensible en logs (reglas de [Logging](../07-observability/logging.md)).
5. Errores con códigos del [catálogo](../03-tools/tool-contract.md#errores); ningún stack trace hacia el Cloud.
6. Documentación actualizada si la implementación obligó a cambiar el diseño (incluida [Cloud Compatibility](../00-overview/cloud-compatibility.md) si afecta al contrato).
7. Revisión de código por otra persona (o autorrevisión con checklist si se trabaja solo).
8. Sin trabajo pendiente oculto: todo lo aplazado queda como HU nueva o nota en la HU.

## Índice

| ID | Título | Epic | Fase |
|---|---|---|---|
| [HU-WIN-001](stories/EPIC-01-agent-foundation.md#hu-win-001--workspace-cargo-crates-y-ci) | Workspace Cargo, crates y CI | EPIC-01 | P1 |
| [HU-WIN-002](stories/EPIC-01-agent-foundation.md#hu-win-002--carga-de-configuración-configtoml) | Carga de configuración | EPIC-01 | P1 |
| [HU-WIN-003](stories/EPIC-01-agent-foundation.md#hu-win-003--logging-estructurado-con-redacción) | Logging estructurado con redacción | EPIC-01 | P1 |
| [HU-WIN-004](stories/EPIC-01-agent-foundation.md#hu-win-004--ciclo-de-vida-del-proceso) | Ciclo de vida del proceso | EPIC-01 | P1 |
| [HU-WIN-005](stories/EPIC-01-agent-foundation.md#hu-win-005--cli-y-canal-local-named-pipe) | CLI y canal local | EPIC-01 | P1 |
| [HU-WIN-006](stories/EPIC-02-cloud-connectivity.md#hu-win-006--crate-jarvis-protocol-y-contract-tests) | Crate `jarvis-protocol` y contract tests | EPIC-02 | P2 |
| [HU-WIN-007](stories/EPIC-02-cloud-connectivity.md#hu-win-007--conexión-mqtttls) | Conexión MQTT/TLS | EPIC-02 | P2 |
| [HU-WIN-008](stories/EPIC-02-cloud-connectivity.md#hu-win-008--presencia-hello-heartbeat-goodbye) | Presencia | EPIC-02 | P2 |
| [HU-WIN-009](stories/EPIC-02-cloud-connectivity.md#hu-win-009--reconexión-y-backoff) | Reconexión y backoff | EPIC-02 | P2 |
| [HU-WIN-010](stories/EPIC-02-cloud-connectivity.md#hu-win-010--validación-de-mensajes-entrantes) | Validación de mensajes entrantes | EPIC-02 | P2 |
| [HU-WIN-011](stories/EPIC-02-cloud-connectivity.md#hu-win-011--estado-de-la-sesión-de-windows) | Estado de la sesión de Windows | EPIC-02 | P2 |
| [HU-WIN-012](stories/EPIC-03-device-identity.md#hu-win-012--adaptador-de-credential-manager) | Adaptador de Credential Manager | EPIC-03 | P2 |
| [HU-WIN-013](stories/EPIC-03-device-identity.md#hu-win-013--comando-provision) | Comando `provision` | EPIC-03 | P2 |
| [HU-WIN-014](stories/EPIC-03-device-identity.md#hu-win-014--uso-de-la-credencial-en-el-cloud-link) | Uso de la credencial en el Cloud Link | EPIC-03 | P2 |
| [HU-WIN-015](stories/EPIC-04-local-state.md#hu-win-015--sqlite-store) | SQLite Store | EPIC-04 | P3 |
| [HU-WIN-016](stories/EPIC-04-local-state.md#hu-win-016--registro-de-tareas-y-deduplicación) | Registro de tareas y deduplicación | EPIC-04 | P3 |
| [HU-WIN-017](stories/EPIC-04-local-state.md#hu-win-017--outbox-y-settle) | Outbox y settle | EPIC-04 | P3 |
| [HU-WIN-018](stories/EPIC-04-local-state.md#hu-win-018--recuperación-de-arranque) | Recuperación de arranque | EPIC-04 | P3 |
| [HU-WIN-019](stories/EPIC-04-local-state.md#hu-win-019--retención-y-redacción-post-settle) | Retención y redacción post-settle | EPIC-04 | P4 |
| [HU-WIN-020](stories/EPIC-05-tool-system.md#hu-win-020--contrato-de-tool-y-toolresult) | Contrato de Tool y ToolResult | EPIC-05 | P4 |
| [HU-WIN-021](stories/EPIC-05-tool-system.md#hu-win-021--registro-de-tools-y-mapeo-tasktype) | Registro de tools y mapeo TaskType | EPIC-05 | P4 |
| [HU-WIN-022](stories/EPIC-05-tool-system.md#hu-win-022--validación-de-input-y-tool-noop) | Validación de input y tool `noop` | EPIC-05 | P4 |
| [HU-WIN-023](stories/EPIC-05-tool-system.md#hu-win-023--executor-scheduler-locks-timeouts-y-cancelación) | Executor | EPIC-05 | P4 |
| [HU-WIN-024](stories/EPIC-06-permission-engine.md#hu-win-024--modelo-y-carga-de-policytoml) | Modelo y carga de `policy.toml` | EPIC-06 | P4 |
| [HU-WIN-025](stories/EPIC-06-permission-engine.md#hu-win-025--evaluación-de-riesgo) | Evaluación de riesgo | EPIC-06 | P4 |
| [HU-WIN-026](stories/EPIC-06-permission-engine.md#hu-win-026--evaluación-de-permisos-y-policy-check) | Evaluación de permisos y `policy check` | EPIC-06 | P4 |
| [HU-WIN-027](stories/EPIC-06-permission-engine.md#hu-win-027--aprobaciones-locales) | Aprobaciones locales | EPIC-06 | P4 |
| [HU-WIN-028](stories/EPIC-07-process-tools.md#hu-win-028--plataforma-procesos-y-ventanas) | Plataforma: procesos y ventanas | EPIC-07 | P5 |
| [HU-WIN-029](stories/EPIC-07-process-tools.md#hu-win-029--catálogo-de-aplicaciones) | Catálogo de aplicaciones | EPIC-07 | P5 |
| [HU-WIN-030](stories/EPIC-07-process-tools.md#hu-win-030--tool-is_app_running) | `is_app_running` | EPIC-07 | P5 |
| [HU-WIN-031](stories/EPIC-07-process-tools.md#hu-win-031--tool-list_running_apps) | `list_running_apps` | EPIC-07 | P5 |
| [HU-WIN-032](stories/EPIC-07-process-tools.md#hu-win-032--tool-get_active_window) | `get_active_window` | EPIC-07 | P5 |
| [HU-WIN-033](stories/EPIC-07-process-tools.md#hu-win-033--tool-open_app--alias-open_chrome) | `open_app` + alias | EPIC-07 | P5 |
| [HU-WIN-034](stories/EPIC-07-process-tools.md#hu-win-034--tool-close_app) | `close_app` | EPIC-07 | P5 |
| [HU-WIN-035](stories/EPIC-08-verification.md#hu-win-035--verification-engine) | Verification Engine | EPIC-08 | P4 |
| [HU-WIN-036](stories/EPIC-08-verification.md#hu-win-036--métodos-process_and_window-y-process_absent) | Métodos de verificación de procesos | EPIC-08 | P5 |
| [HU-WIN-037](stories/EPIC-08-verification.md#hu-win-037--verificación-reflejada-en-resultados-y-mensajes) | Verificación en resultados y mensajes | EPIC-08 | P5 |
| [HU-WIN-038](stories/EPIC-09-task-execution.md#hu-win-038--intake-de-tareas-etapas-17-y-11) | Intake de tareas | EPIC-09 | P3 |
| [HU-WIN-039](stories/EPIC-09-task-execution.md#hu-win-039--ejecución-y-cierre-etapas-1217) | Ejecución y cierre | EPIC-09 | P4 |
| [HU-WIN-040](stories/EPIC-09-task-execution.md#hu-win-040--cancelación-de-tareas) | Cancelación | EPIC-09 | P4 |
| [HU-WIN-041](stories/EPIC-09-task-execution.md#hu-win-041--aprobación-dentro-del-pipeline) | Aprobación en el pipeline | EPIC-09 | P4 |
| [HU-WIN-042](stories/EPIC-09-task-execution.md#hu-win-042--vertical-slice-con-jarvis-cloud) | Vertical slice | EPIC-09 | P5 |
| [HU-WIN-043](stories/EPIC-10-resilience.md#hu-win-043--corrupción-de-sqlite-y-guardia-de-reinicio) | Corrupción de SQLite | EPIC-10 | P6 |
| [HU-WIN-044](stories/EPIC-10-resilience.md#hu-win-044--suite-de-escenarios-de-fallo-chaos) | Suite de fallos | EPIC-10 | P6 |
| [HU-WIN-045](stories/EPIC-10-resilience.md#hu-win-045--protección-de-recursos-y-tareas-atascadas) | Protección de recursos | EPIC-10 | P6 |
| [HU-WIN-046](stories/EPIC-11-security.md#hu-win-046--canarios-de-datos-sensibles) | Canarios de datos sensibles | EPIC-11 | P6 |
| [HU-WIN-047](stories/EPIC-11-security.md#hu-win-047--fuzzing-del-parser) | Fuzzing del parser | EPIC-11 | P6 |
| [HU-WIN-048](stories/EPIC-11-security.md#hu-win-048--endurecimiento-del-proceso) | Endurecimiento del proceso | EPIC-11 | P6 |
| [HU-WIN-049](stories/EPIC-11-security.md#hu-win-049--suite-de-seguridad-del-threat-model) | Suite de seguridad | EPIC-11 | P6 |
| [HU-WIN-050](stories/EPIC-12-packaging.md#hu-win-050--install--uninstall) | `install` / `uninstall` | EPIC-12 | P6 |
| [HU-WIN-051](stories/EPIC-12-packaging.md#hu-win-051--build-de-release-firmada) | Release firmada | EPIC-12 | P6 |
| [HU-WIN-052](stories/EPIC-12-packaging.md#hu-win-052--comando-diagnostics) | `diagnostics` | EPIC-12 | P6 |
| [HU-WIN-053](stories/EPIC-12-packaging.md#hu-win-053--aceptación-del-mvp) | Aceptación del MVP | EPIC-12 | P6 |
| [HU-WIN-054](stories/EPIC-13-post-mvp-tools.md#hu-win-054--sandbox-de-rutas) | Sandbox de rutas | EPIC-13 | P7 |
| [HU-WIN-055](stories/EPIC-13-post-mvp-tools.md#hu-win-055--tool-list_directory) | `list_directory` | EPIC-13 | P7 |
| [HU-WIN-056](stories/EPIC-13-post-mvp-tools.md#hu-win-056--tool-read_file) | `read_file` | EPIC-13 | P7 |
| [HU-WIN-057](stories/EPIC-13-post-mvp-tools.md#hu-win-057--catálogo-de-comandos-y-lanzador-con-job-object) | Catálogo de comandos y Job Objects | EPIC-13 | P7 |
| [HU-WIN-058](stories/EPIC-13-post-mvp-tools.md#hu-win-058--tool-run_command) | `run_command` | EPIC-13 | P7 |
| [HU-WIN-059](stories/EPIC-13-post-mvp-tools.md#hu-win-059--tool-focus_window) | `focus_window` | EPIC-13 | P7 |
| [HU-WIN-060](stories/EPIC-13-post-mvp-tools.md#hu-win-060--tool-show_notification) | `show_notification` | EPIC-13 | P7 |
| [HU-WIN-061](stories/EPIC-14-signed-approvals-updates.md#hu-win-061--verificación-de-firmas-de-tareas) | Firmas de tareas | EPIC-14 | P8 |
| [HU-WIN-062](stories/EPIC-14-signed-approvals-updates.md#hu-win-062--aprobación-remota-firmada) | Aprobación remota firmada | EPIC-14 | P8 |
| [HU-WIN-063](stories/EPIC-14-signed-approvals-updates.md#hu-win-063--actualizador-firmado-con-rollback) | Updater firmado | EPIC-14 | P8 |
| [HU-WIN-064](stories/EPIC-15-ui-automation.md#hu-win-064--host-uia-en-hilo-sta) | Host UIA | EPIC-15 | P9 |
| [HU-WIN-065](stories/EPIC-15-ui-automation.md#hu-win-065--find_window-y-find_element) | `find_window` / `find_element` | EPIC-15 | P9 |
| [HU-WIN-066](stories/EPIC-15-ui-automation.md#hu-win-066--invoke_element-set_text-get_text) | `invoke_element` / `set_text` / `get_text` | EPIC-15 | P9 |

La columna **Fase** es la del [orden de implementación](implementation-order.md) y puede diferir de la fase nominal de su épica (p. ej. HU-WIN-035 se adelanta a P4 porque el pipeline la necesita).
