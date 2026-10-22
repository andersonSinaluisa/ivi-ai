# Data Flow

Flujos principales de extremo a extremo. Los mensajes MQTT se detallan en [MQTT Protocol](../03-protocols/mqtt-protocol.md) y los estados en [Task State Machine](../02-domain/task-state-machine.md). En los diagramas, `{src}` y `{dst}` son los `deviceId` del ESP32 y del Windows device.

## 1. Registro de un device

```mermaid
sequenceDiagram
    autonumber
    actor Owner
    participant API as jarvis-api
    participant PG as PostgreSQL
    participant MQ as Mosquitto (dynsec)
    Owner->>API: POST /api/v1/devices (JWT de User)
    API->>API: genera deviceId (UUIDv7) y secreto 256 bits
    API->>PG: BEGIN - INSERT device, device_credential (hash), event DEVICE_REGISTERED
    API->>MQ: CONTROL createClient(username=deviceId, clientid=deviceId, password, role=jarvis-device)
    alt broker responde OK
        API->>PG: broker_synced_at = now - COMMIT
    else broker no disponible o error
        API->>PG: ROLLBACK
        API-->>Owner: 503 SERVICE_UNAVAILABLE (no se creó nada, reintentar)
    end
    API-->>Owner: 201 con deviceId, secret (única vez), mqtt host/port
    Owner->>Owner: provisiona el device (NVS, Credential Manager, archivo)
```

## 2. Caso normal: ESP32 pide OPEN_CHROME al Windows device

```mermaid
sequenceDiagram
    autonumber
    participant E as ESP32 Simulator
    participant MQ as Mosquitto
    participant API as jarvis-api
    participant PG as PostgreSQL
    participant W as Windows Simulator
    E->>MQ: PUBLISH up/requests task.create (idempotencyKey, target=dst, OPEN_CHROME)
    MQ->>API: entrega (suscripción up/#)
    API->>PG: valida grant, idempotencia, target soporta tipo - INSERT task Pending + TASK_CREATED
    API->>MQ: PUBLISH down/notifications request.accepted (taskId) a src
    MQ->>E: request.accepted
    API->>PG: dispatcher: claim Pending (SKIP LOCKED) - Dispatched + TASK_DISPATCHED
    API->>MQ: PUBLISH down/tasks task.dispatch a dst
    MQ->>W: task.dispatch
    W->>W: dedup por taskId, persiste en inbox
    W->>MQ: up/tasks task.ack
    MQ->>API: task.ack
    API->>PG: Delivered + TASK_DELIVERED
    API->>MQ: down/notifications task.update Delivered a src
    W->>MQ: up/tasks task.started
    MQ->>API: task.started
    API->>PG: Running + TASK_STARTED
    API->>MQ: task.update Running a src
    W->>W: simula ejecución
    W->>MQ: up/tasks task.completed (result)
    MQ->>API: task.completed
    API->>PG: Completed + result + TASK_COMPLETED
    API->>MQ: down/tasks task.update.ack a dst
    MQ->>W: task.update.ack - W marca la tarea settled
    API->>MQ: down/notifications task.update Completed + result a src
    MQ->>E: task.update Completed
```

En paralelo, cada cambio de estado emite una notificación SignalR al grupo del owner (no mostrado).

## 3. Target offline

```mermaid
sequenceDiagram
    autonumber
    participant E as ESP32
    participant API as jarvis-api
    participant PG as PostgreSQL
    participant W as Windows device
    E->>API: task.create (vía MQTT)
    API->>PG: INSERT task Pending (target Offline)
    API-->>E: request.accepted (status Pending)
    Note over API,PG: El dispatcher ignora targets Offline. La tarea espera hasta expires_at.
    W->>API: CONNECT + presence.hello (openTaskIds)
    API->>PG: presence Online, reconciliación de tareas abiertas
    API->>PG: dispatcher despertado: claim Pending de dst
    API->>W: task.dispatch
    W-->>API: task.ack ... (sigue como caso normal)
    Note over API,PG: Si expires_at vence antes de conectar: Expired + TASK_EXPIRED y task.update a E
```

## 4. Reconexión durante la entrega (evitar doble ejecución)

```mermaid
sequenceDiagram
    autonumber
    participant API as jarvis-api
    participant MQ as Mosquitto
    participant W as Windows device
    API->>MQ: task.dispatch Task A (attempt 1)
    MQ->>W: task.dispatch Task A
    W->>W: persiste A en inbox (known, Delivered)
    W--xMQ: conexión perdida antes de que llegue task.ack
    Note over API: A sigue Dispatched
    W->>MQ: CONNECT (clean start) + LWT nueva sesión
    W->>MQ: presence.hello openTaskIds=[A]
    MQ->>API: hello
    API->>API: A está en openTaskIds, no se revierte a Pending
    W->>MQ: reenvía estado actual de A desde su outbox (task.ack o posterior)
    MQ->>API: task.ack A
    API->>API: Dispatched a Delivered
    Note over API,W: Si el sweeper hubiera redespachado A (attempt 2), W la reconoce por taskId, NO la ejecuta y responde con su estado actual.
```

Garantía: **la doble ejecución se evita en el device** (consumidor idempotente con registro durable de `taskId`), no en el transporte. El Cloud solo garantiza at-least-once. Ver [ADR-006](../08-decisions/ADR-006-task-delivery-semantics.md).

## 5. Pérdida del device durante la ejecución

1. W reporta `task.started` (Running) y se apaga sin enviar resultado.
2. El broker publica el LWT → `DEVICE_DISCONNECTED`. La tarea **no** cambia (puede seguir corriendo si solo se cayó la red).
3. Al reconectar, W envía `presence.hello` con `openTaskIds`:
   - Si incluye A → el Cloud espera el resultado (W reenvía su outbox).
   - Si no incluye A → `Failed` con `error.code = DEVICE_LOST`.
4. Si W nunca vuelve y la tarea tiene `max_duration_seconds`, el `TaskExpirySweeper` la marca `Failed EXECUTION_TIMEOUT`. Sin `max_duration_seconds`, se aplica el máximo del sistema (24 h).

## 6. Cancelación

| Estado actual | Efecto de `POST /api/v1/tasks/{id}/cancel` |
|---|---|
| `Pending` | `Cancelled` inmediato. Si `attempt_count > 0` (fue despachada antes y revertida), además se publica `task.cancel` best effort: el device la registra como cancelada y rechaza cualquier redelivery; si ya la había ejecutado, su resultado llega como `late_terminal_update`. |
| `Dispatched`, `Delivered`, `Running`, `WaitingForApproval` | Se registra `cancel_requested_at`, evento `TASK_CANCEL_REQUESTED`, se publica `task.cancel`. El estado final lo decide el device: `task.cancelled` → `Cancelled`; si ya había terminado, su `task.completed`/`task.failed` prevalece. |
| Terminal | `409 INVALID_STATE_TRANSITION`. |

## 7. Trazabilidad

Cada flujo conserva un único `correlationId`: se genera (o se acepta del cliente) al crear la tarea, viaja en todos los envelopes MQTT, se guarda en `tasks.correlation_id` y en cada fila de `events`, y aparece en todos los logs. Con él se reconstruye: quién creó (`requested_by_*`), desde qué device (`source_device_id`), hacia cuál (`target_device_id`), y las marcas `created_at`, `dispatched_at`, `delivered_at`, `started_at`, `completed_at` más `result`/`error_*`.
