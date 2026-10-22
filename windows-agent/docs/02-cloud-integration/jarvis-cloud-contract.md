# Contrato con Jarvis Cloud

**Fuente de verdad:** [api/docs/03-protocols/mqtt-protocol.md](../../../api/docs/03-protocols/mqtt-protocol.md) (MQTT v1) y [api/docs/07-integrations/windows-agent-contract.md](../../../api/docs/07-integrations/windows-agent-contract.md). Este documento dice **cómo** el agente cumple ese contrato; no lo redefine. Diferencias: [Cloud Compatibility](../00-overview/cloud-compatibility.md).

## Lo que el agente recibe (`jarvis/v1/devices/{self}/down/#`)

| Topic | Tipo | Uso en el agente |
|---|---|---|
| `down/tasks` | `task.dispatch` | Entrada al [pipeline](../01-architecture/execution-pipeline.md) |
| `down/tasks` | `task.cancel` | [Cancelación](../01-architecture/execution-pipeline.md#cancelación) |
| `down/tasks` | `task.update.ack` | Marcar la tarea *settled* y dejar de reenviar |
| `down/control` | `presence.hello.ack` | Conexión lista: `heartbeatIntervalSeconds`, `serverTime` (desfase de reloj), `maxMessageBytes` |
| `down/notifications` | `request.*`, `task.update` | El agente no origina tareas en MVP: se registran en `DEBUG` y se ignoran |

## Lo que el agente publica (`jarvis/v1/devices/{self}/up/…`)

| Topic | Tipo | Cuándo | QoS |
|---|---|---|---|
| `up/presence` | `presence.hello` | Tras cada `CONNECT` | 1 |
| `up/presence` | `presence.heartbeat` | Cada `heartbeatIntervalSeconds` | 0 |
| `up/presence` | `presence.goodbye` | Apagado ordenado | 1 |
| `up/presence` | `presence.offline` (LWT) | Lo publica el broker si el agente cae | 1 |
| `up/tasks` | `task.ack`, `task.rejected`, `task.started`, `task.progress`, `task.approval_required`, `task.completed`, `task.failed`, `task.cancelled` | Ciclo de vida | 1 |
| `up/requests` | `task.create`, `task.get` | **No se usa** en MVP (el agente no origina tareas) | — |
| `up/events` | `device.event` | **No se usa** (reservado por el Cloud) | — |

## Cumplimiento de W1–W10

| Obligación del Cloud | Cómo la cumple el agente | Documento |
|---|---|---|
| W1 Conexión (clientId = username = deviceId, TLS, clean start, keepalive 60, LWT con sessionId) | Cloud Link | [MQTT Client](mqtt-client.md) |
| W2 `down/#` + hello con `openTaskIds`, `supportedTaskTypes`, capabilities | Cloud Link + Tool Host + State Store | [Presence](presence.md) |
| W3 Heartbeat con `busy` | Cloud Link + Executor | [Presence](presence.md) |
| W4 Dedup, expiración, NOT_PERMITTED, persistir y ack | Etapas 4–11 del pipeline | [Execution Pipeline](../01-architecture/execution-pipeline.md) |
| W5 Persistir antes de cada efecto | Transacciones SQLite antes de cada publicación y antes de ejecutar | [Local State](../06-state/local-state.md) |
| W6 Reenvío hasta `task.update.ack` | Outbox | [Idempotency](../06-state/idempotency.md) |
| W7 `taskId` ≥ 8 días | Retención de 8 días tras *settled* | [Idempotency](../06-state/idempotency.md#retención) |
| W8 `task.cancel` | Cancelación por tool | [Execution Pipeline](../01-architecture/execution-pipeline.md#cancelación) |
| W9 Tarea interrumpida → failed sin re-ejecutar | Recuperación de arranque (`EXECUTION_INTERRUPTED`, ver C6) | [Crash Recovery](../06-state/crash-recovery.md) |
| W10 Contenido no confiable | Esquemas estrictos, sin shell, catálogo, política | [Security Architecture](../04-security/security-architecture.md) |

## Mapeo de identificadores

| Concepto | Origen | Uso local |
|---|---|---|
| `deviceId` | Cloud (registro) | Username/clientId MQTT, clave de Credential Manager, validación de topics |
| `taskId` | Cloud | Clave de idempotencia local (única) |
| `correlationId` | Cloud (tarea) | En todos los mensajes `up/tasks` de la tarea y en logs |
| `executionId` | Agente (UUIDv7) | Identifica la ejecución local; va en `result` y logs |
| `idempotencyKey` | Cloud (creación de la tarea) | **No** se usa en el agente: el Cloud ya lo convirtió en un único `taskId` |
| `sessionId` | Agente (por conexión) | LWT y presencia |
