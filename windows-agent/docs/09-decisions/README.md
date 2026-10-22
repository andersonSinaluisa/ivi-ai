# Architecture Decision Records

Formato: **Status · Context · Decision · Alternatives · Consequences**. Una ADR aceptada solo se cambia con otra ADR que la reemplace (`Superseded by`).

| ADR | Título | Status |
|---|---|---|
| [ADR-001](ADR-001-rust-agent-runtime.md) | Rust como runtime del agente | Accepted |
| [ADR-002](ADR-002-windows-integration-strategy.md) | Estrategia jerárquica de integración con Windows | Accepted |
| [ADR-003](ADR-003-tool-abstraction.md) | Abstracción de Tool con contrato explícito | Accepted |
| [ADR-004](ADR-004-local-permission-authority.md) | Autoridad de permisos local | Accepted |
| [ADR-005](ADR-005-sqlite-local-state.md) | SQLite para el estado local | Accepted |
| [ADR-006](ADR-006-task-idempotency.md) | Idempotencia por `taskId` | Accepted |
| [ADR-007](ADR-007-execution-vs-verification.md) | Separar ejecución y verificación | Accepted |
| [ADR-008](ADR-008-restricted-command-execution.md) | Ejecución de comandos restringida | Accepted |
| [ADR-009](ADR-009-credential-storage.md) | Credenciales en Windows Credential Manager | Accepted |
| [ADR-010](ADR-010-agent-startup-model.md) | Scheduled Task por usuario al iniciar sesión | Accepted |
| [ADR-011](ADR-011-privilege-model.md) | Usuario estándar, sin elevación | Accepted |
| [ADR-012](ADR-012-ui-automation-strategy.md) | UI Automation como mecanismo de UI | Accepted (implementación P9) |
| [ADR-013](ADR-013-browser-automation-boundary.md) | Navegador mediante provider Playwright separado | Accepted (implementación P11) |
| [ADR-014](ADR-014-agent-core-boundary.md) | Boundary con el Agent Core y BrainProvider | Accepted (implementación P10) |
| [ADR-015](ADR-015-task-type-tool-mapping.md) | Mapeo TaskType ↔ Tool | Accepted |
| [ADR-016](ADR-016-mqtt-transport-and-loopback.md) | MQTT 3.1.1 y excepción de texto plano en loopback | Accepted |
| [ADR-017](ADR-017-notification-mechanism.md) | Notificaciones por globo del área de notificación | Accepted |

ADR-015 se añade a las 14 pedidas porque el mapeo entre el catálogo de tipos del Cloud y las tools es un punto de compatibilidad que conviene fijar por escrito ([Cloud Compatibility](../00-overview/cloud-compatibility.md)). ADR-016 documenta dos ajustes de transporte que surgieron al implementar el Cloud Link, y ADR-017 el mecanismo real de notificación tras implementar P7.
