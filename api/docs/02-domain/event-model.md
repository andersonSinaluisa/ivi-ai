# Event Model

## Qué es y qué no es

- **Es** un *event log operacional*: registro append-only de hechos relevantes (auditoría, depuración, catch-up de clientes realtime).
- **No es** Event Sourcing: el estado actual vive en las tablas (`tasks`, `devices`); los eventos no se reproducen para reconstruirlo.
- Se escribe **en la misma transacción** que el cambio de estado que describe. No puede existir un cambio de estado sin su evento ni viceversa.

## Estructura

| Campo | Tipo | Notas |
|---|---|---|
| `seq` | bigint identity | Orden total global, usado como cursor (`after`). |
| `id` | UUIDv7 | Identificador público. |
| `owner_id` | UUID? | Nulo solo para eventos de sistema sin owner (p. ej. auth fallida con email inexistente). |
| `type` | text | `UPPER_SNAKE_CASE`, catálogo abajo. |
| `occurred_at` | timestamptz | Momento del hecho (puede venir del device, validado ±5 min; si no, hora del servidor). |
| `recorded_at` | timestamptz | Hora del servidor al insertar. |
| `correlation_id` | UUID | |
| `actor_type` | `User` \| `Device` \| `System` | Quién causó el hecho. |
| `actor_id` | UUID? | |
| `device_id` | UUID? | Device afectado. |
| `task_id` | UUID? | Tarea afectada. |
| `data` | jsonb | Detalle específico del tipo; **pequeño** (≤ 4 KiB) y **sin secretos, payloads ni results completos**. |

## Catálogo

### Devices

| Type | Actor | `data` |
|---|---|---|
| `DEVICE_REGISTERED` | User | `{ "name", "type" }` |
| `DEVICE_UPDATED` | User | `{ "changes": ["name"] }` |
| `DEVICE_DISABLED` / `DEVICE_ENABLED` | User | `{}` |
| `DEVICE_REVOKED` | User | `{ "cancelledTasks": 2 }` |
| `DEVICE_CREDENTIAL_ROTATED` | User | `{ "credentialId", "hint" }` |
| `DEVICE_CONNECTED` | Device | `{ "sessionId", "reason": "hello" \| "recovered", "softwareVersion", "protocolVersion" }` |
| `DEVICE_DISCONNECTED` | Device/System | `{ "sessionId", "reason": "goodbye" \| "lwt" \| "heartbeat_timeout" \| "disabled" }` |
| `DEVICE_AUTH_FAILED` | System | `{ "deviceId", "channel": "rest" }` (solo REST; los fallos MQTT quedan en logs de Mosquitto) |

### Tasks

| Type | Actor | `data` |
|---|---|---|
| `TASK_CREATED` | User/Device | `{ "type", "targetDeviceId", "channel": "rest" \| "mqtt", "expiresAt" }` |
| `TASK_REJECTED` | User/Device | `{ "type", "targetDeviceId", "code" }` (creación rechazada por autorización/validación de negocio; `task_id` nulo) |
| `TASK_DISPATCHED` | System | `{ "attempt": 1 }` |
| `TASK_DELIVERED` | Device | `{}` |
| `TASK_STARTED` | Device | `{}` |
| `TASK_COMPLETED` | Device | `{ "resultSizeBytes": 42 }` |
| `TASK_FAILED` | Device/System | `{ "code": "EXECUTION_TIMEOUT" }` |
| `TASK_CANCEL_REQUESTED` | User/Device | `{}` |
| `TASK_CANCELLED` | User/Device/System | `{ "reason": "USER_REQUEST" \| "APPROVAL_DENIED" \| "LOCAL_POLICY" \| "DEVICE_REVOKED" }` |
| `TASK_EXPIRED` | System | `{ "lastStatus": "Dispatched" }` |

### Approvals

| Type | Actor | `data` |
|---|---|---|
| `APPROVAL_REQUESTED` | Device | `{ "reason": "texto corto del device" }` |
| `APPROVAL_DECISION_SENT` | User (o Device) | `{ "decision": "GRANTED" \| "DENIED", "nonce", "signatureKeyId", "decidedAt", "channel": "rest" }` |
| `APPROVAL_GRANTED` | Device | `{ "decidedBy": "device" }` |
| `APPROVAL_DENIED` | Device | `{ "decidedBy": "device" }` |

`APPROVAL_DECISION_SENT` y `APPROVAL_GRANTED` son **hechos distintos** y por eso son eventos distintos: el primero dice que el owner firmó una decisión y el Cloud la transmitió; el segundo, que el device la aplicó de verdad. Una decisión con firma inválida, caducada o repetida deja el primero sin el segundo, y eso es exactamente lo que se quiere poder ver en la auditoría ([ADR-012](../08-decisions/ADR-012-end-to-end-task-signing.md)).

### Identity y seguridad

| Type | Actor | `data` |
|---|---|---|
| `USER_CREATED` | System | `{ "via": "bootstrap-owner" }` |
| `USER_LOGIN_SUCCEEDED` | User | `{ "ip": "hash" }` |
| `USER_LOGIN_FAILED` | System | `{ "reason": "bad_password" \| "locked" \| "unknown_user" }` |
| `USER_LOCKED_OUT` | System | `{ "until" }` |
| `USER_PASSWORD_RESET` | System | `{ "via": "reset-owner-password" }` |
| `GRANT_CREATED` / `GRANT_DELETED` | User | `{ "sourceDeviceId", "targetDeviceId", "taskType" }` |
| `SECURITY_VIOLATION` | Device/User | `{ "rule": "topic_device_mismatch" \| "not_task_target" \| "unknown_task" \| "task_not_dispatched" \| "message_flood" }` |

Los IPs se registran como hash con sal diaria (HMAC) para permitir correlación sin almacenar el dato personal en claro.

## Lo que NO es un evento

- Heartbeats (solo actualizan `last_seen_at`).
- `task.progress` (actualiza columnas de progreso; generaría ruido).
- Transiciones `Online ↔ Stale` (solo notificación realtime).
- Lecturas (GET) de la API.

## Mapeo a notificaciones realtime

| Event | Notificación SignalR |
|---|---|
| `DEVICE_REGISTERED` | `device.registered` |
| `DEVICE_UPDATED`, `DEVICE_DISABLED`, `DEVICE_ENABLED`, `DEVICE_REVOKED` | `device.updated` |
| `DEVICE_CONNECTED` | `device.online` |
| `DEVICE_DISCONNECTED` | `device.offline` |
| (transición a `Stale`, sin evento) | `device.stale` |
| `TASK_CREATED` | `task.created` |
| `TASK_DISPATCHED` | `task.dispatched` |
| `TASK_DELIVERED` | `task.delivered` |
| `TASK_STARTED` | `task.started` |
| `TASK_COMPLETED` | `task.completed` |
| `TASK_FAILED` | `task.failed` |
| `TASK_CANCELLED` | `task.cancelled` |
| `TASK_EXPIRED` | `task.expired` |
| `APPROVAL_REQUESTED` | `approval.requested` |
| `APPROVAL_GRANTED`, `APPROVAL_DENIED` | `approval.resolved` |
| Resto | No se notifica |

Contrato completo: [Realtime Protocol](../03-protocols/realtime-protocol.md).

## Retención

- 180 días por defecto (configurable). Job diario borra por lotes filas con `recorded_at` anterior. Única operación de borrado permitida, ejecutada con un rol distinto (`jarvis_maintenance`).
- Evolución: particionamiento mensual por `recorded_at` cuando la tabla supere ~10 M filas.
