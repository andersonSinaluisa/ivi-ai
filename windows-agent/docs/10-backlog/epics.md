# Epics

| Epic | Nombre | Fase | Objetivo | HUs | Archivo |
|---|---|---|---|---|---|
| EPIC-01 | Agent Foundation | P1 | Proceso Rust estable, configurable, con logs y CLI | HU-WIN-001 … 005 | [stories/EPIC-01](stories/EPIC-01-agent-foundation.md) |
| EPIC-02 | Cloud Connectivity | P2 | Conexión MQTT/TLS, presencia, reconexión, validación de entrada, estado de sesión | HU-WIN-006 … 011 | [stories/EPIC-02](stories/EPIC-02-cloud-connectivity.md) |
| EPIC-03 | Device Identity | P2 | Credencial en Credential Manager, `provision`, estados de identidad | HU-WIN-012 … 014 | [stories/EPIC-03](stories/EPIC-03-device-identity.md) |
| EPIC-04 | Local State | P3 | SQLite, dedup, outbox, recuperación, retención | HU-WIN-015 … 019 | [stories/EPIC-04](stories/EPIC-04-local-state.md) |
| EPIC-05 | Tool System | P4 | Contrato, registro, validación, executor con locks/timeouts/cancelación | HU-WIN-020 … 023 | [stories/EPIC-05](stories/EPIC-05-tool-system.md) |
| EPIC-06 | Permission Engine | P4 | Política, riesgo, decisiones, aprobaciones locales | HU-WIN-024 … 027 | [stories/EPIC-06](stories/EPIC-06-permission-engine.md) |
| EPIC-07 | Process Tools | P5 | Plataforma de procesos/ventanas, catálogo de apps y las 5 tools del MVP | HU-WIN-028 … 034 | [stories/EPIC-07](stories/EPIC-07-process-tools.md) |
| EPIC-08 | Verification | P4–P5 | Motor de verificación y métodos de procesos | HU-WIN-035 … 037 | [stories/EPIC-08](stories/EPIC-08-verification.md) |
| EPIC-09 | Task Execution | P3–P5 | Pipeline de 17 etapas, cancelación, aprobaciones, vertical slice | HU-WIN-038 … 042 | [stories/EPIC-09](stories/EPIC-09-task-execution.md) |
| EPIC-10 | Resilience | P6 | Corrupción de BD, suite de fallos, protección de recursos | HU-WIN-043 … 045 | [stories/EPIC-10](stories/EPIC-10-resilience.md) |
| EPIC-11 | Security | P6 | Canarios, fuzzing, endurecimiento, suite del threat model | HU-WIN-046 … 049 | [stories/EPIC-11](stories/EPIC-11-security.md) |
| EPIC-12 | Packaging | P6 | Instalación, release firmada, diagnóstico, aceptación del MVP | HU-WIN-050 … 053 | [stories/EPIC-12](stories/EPIC-12-packaging.md) |
| EPIC-13 | Post-MVP Tools | P7 | Sandbox de rutas, `list_directory`, `read_file`, comandos con nombre, `run_command`, `focus_window`, `show_notification` | HU-WIN-054 … 060 | [stories/EPIC-13](stories/EPIC-13-post-mvp-tools.md) |
| EPIC-14 | Firmas, aprobaciones remotas y updates | P8 | Firma de tareas, aprobación remota firmada, updater con rollback | HU-WIN-061 … 063 | [stories/EPIC-14](stories/EPIC-14-signed-approvals-updates.md) |
| EPIC-15 | UI Automation | P9 | Host UIA, búsqueda e interacción semántica | HU-WIN-064 … 066 | [stories/EPIC-15](stories/EPIC-15-ui-automation.md) |
| EPIC-16 | Local Tool API | P10 | `tools/list`, `tools/call` por named pipe para el Agent Core | Implementada (sin HU numeradas) | [Agent Core Integration](../08-future/agent-core-integration.md) |
| EPIC-17 | Browser Provider | P11 | Provider Playwright separado | Por detallar | [Browser Automation](../08-future/browser-automation.md) |
| EPIC-18 | Vision Fallback | P12 | Provider de visión como último recurso | Por detallar | [Computer Vision](../08-future/computer-vision.md) |

**MVP** = EPIC-01 … EPIC-12 (HU-WIN-001 … HU-WIN-053). EPIC-13 … 16 están implementadas ([estado por historia](implementation-status.md)). EPIC-17 y EPIC-18 (navegador y visión) quedan fuera del alcance de este proyecto: son providers separados.

Las épicas pedidas eran EPIC-01…12. Se añaden 13–18 para cubrir el roadmap completo.
