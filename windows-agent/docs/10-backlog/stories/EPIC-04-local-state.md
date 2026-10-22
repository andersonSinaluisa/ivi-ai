# EPIC-04 — Local State

Fase **P3**. Objetivo: estado durable que garantice dedup, entrega de resultados y recuperación. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-015 — SQLite Store

**Goal**: base de datos local durable con un único escritor.

**Description**: crate `jarvis-state` según [Local State](../../06-state/local-state.md#ubicación-y-configuración-de-sqlite) y el esquema de [Local State](../../06-state/local-state.md#esquema).

**Acceptance Criteria**
1. Abre o crea `state\agent.db` con `WAL`, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5000`.
2. Migraciones embebidas por `user_version`, idempotentes; tablas `tasks`, `executions` (UNIQUE `task_id`), `approvals`, `outbox`, `meta`.
3. `meta.db_created_at` y `meta.schema_version` al crear.
4. Todas las escrituras pasan por una tarea escritora única (canal de comandos con respuesta); API async para el resto del agente.
5. `PRAGMA quick_check` al abrir; resultado en health (`db.healthy`).
6. Operaciones transaccionales de alto nivel: `record_received`, `transition(task, state, outbox_msgs[])`, etc.; nunca SQL fuera del crate.

**Dependencies**: HU-WIN-001.

**Technical Considerations**: `rusqlite` `bundled`; hilo dedicado con `std::sync::mpsc`/`tokio::sync::mpsc`; fechas en texto RFC 3339 UTC.

**Tests**: unit en memoria y en archivo temporal; migración desde una BD de versión anterior (fixture); test de que dos transacciones concurrentes se serializan.

**Definition of Done**: DoD común.

---

## HU-WIN-016 — Registro de tareas y deduplicación

**Goal**: ningún `taskId` se procesa dos veces.

**Description**: etapa 4 de [Execution Pipeline](../../01-architecture/execution-pipeline.md#etapas) y el algoritmo de [Idempotency](../../06-state/idempotency.md#algoritmo-de-recepción).

**Acceptance Criteria**
1. `taskId` desconocido → se registra como `Received` y continúa el pipeline.
2. `taskId` conocido → se reenvía el último mensaje persistido según su estado (ack / ack+started / terminal) y **no** se ejecuta.
3. Mismo `taskId` con `payload_hash` distinto → se ignora el nuevo y `WARN` con ambos hashes.
4. `task.cancel` de un `taskId` desconocido → tombstone `Cancelled` + `task.cancelled`.
5. 100 dispatches idénticos concurrentes → 1 registro.

**Dependencies**: HU-WIN-015, HU-WIN-006.

**Technical Considerations**: la comprobación y el insert son atómicos en el escritor único (no hay carrera entre dos entregas).

**Tests**: unit con 100 entregas concurrentes; integración con broker: redelivery tras desconexión.

**Definition of Done**: DoD común + [WIN-E2E-03](../../11-testing/integration-scenarios.md#win-e2e-03-tarea-duplicada) verde (cuando exista una tool real).

---

## HU-WIN-017 — Outbox y settle

**Goal**: todo mensaje de estado llega al Cloud al menos una vez, en orden, aunque haya cortes y reinicios.

**Description**: outbox persistente según [Local State](../../06-state/local-state.md#el-caso-crítico) y [MQTT Client](../../02-cloud-integration/mqtt-client.md).

**Acceptance Criteria**
1. Todo mensaje `up/tasks` se inserta en `outbox` en la misma transacción que el cambio de estado que lo origina.
2. Publicación desde el outbox en orden de `seq`, QoS 1; al reconectar se reenvía todo lo no confirmado.
3. `task.update.ack` para un estado terminal → `settled = 1`, `outbox.acked_at`, se deja de reenviar.
4. `openTaskIds` del hello = no terminales + terminales no *settled* (máx. 256, más recientes).
5. Reenvío periódico (cada 60 s) de mensajes sin ack mientras hay conexión, con `attempts` incrementado.

**Dependencies**: HU-WIN-015, HU-WIN-008.

**Technical Considerations**: el PUBACK de MQTT no es el ack de negocio; solo `task.update.ack` asienta.

**Tests**: integración: generar un terminal sin conexión, reiniciar el proceso, conectar → el Cloud recibe el terminal y responde ack.

**Definition of Done**: DoD común + [WIN-E2E-09](../../11-testing/integration-scenarios.md#win-e2e-09-desconexión-durante-la-ejecución) verde.

---

## HU-WIN-018 — Recuperación de arranque

**Goal**: después de cualquier caída, el estado es coherente y nada se re-ejecuta.

**Description**: tabla de [Crash Recovery](../../06-state/crash-recovery.md#recuperación-de-arranque) y modo seguro por crash loop.

**Acceptance Criteria**
1. `Received` sin ack → se elimina.
2. `AwaitingApproval` vencida → `Cancelled` + `task.cancelled APPROVAL_EXPIRED`.
3. `Accepted` → re-encolada (con re-evaluación en el pipeline).
4. `Running`/`Verifying` → `Failed EXECUTION_INTERRUPTED` (`retryable: true`) + `task.failed`.
5. 3 caídas en 10 min → modo seguro: rechaza tareas nuevas, publica pendientes, condición `SAFE_MODE`; `status --clear-safe-mode` lo quita.
6. La recuperación ocurre antes de conectar al Cloud.

**Dependencies**: HU-WIN-016, HU-WIN-017.

**Technical Considerations**: registro de arranques/caídas en `meta` (marcador de "apagado limpio" escrito al cerrar).

**Tests**: unit por estado con estados sembrados en la BD; integración (tras HU-WIN-022): matar el proceso durante una tarea `noop` lenta y reiniciar.

**Definition of Done**: DoD común + [WIN-E2E-10](../../11-testing/integration-scenarios.md#win-e2e-10-reinicio-del-agente-con-trabajo-pendiente) verde.

---

## HU-WIN-019 — Retención y redacción post-settle

**Goal**: la BD local no crece indefinidamente ni guarda datos sensibles más de lo necesario.

**Description**: [Retención](../../06-state/idempotency.md#retención) y redacción de [Execution Pipeline](../../01-architecture/execution-pipeline.md#etapas) (etapa 17).

**Acceptance Criteria**
1. Al quedar *settled*, si la tool declara `sensitivity ≠ none`, `result_json` y `payload_json` se reemplazan por la versión redactada; se conservan `result_sha256` y `result_size`.
2. Limpieza cada hora (y al arrancar): tareas *settled* hace más de 8 días, outbox confirmado hace más de 24 h; en lotes de ≤ 500 filas.
3. Terminales no *settled* nunca se borran; más de 7 días → `OUTBOX_STUCK`.
4. `VACUUM` incremental semanal (`auto_vacuum = INCREMENTAL`).

**Dependencies**: HU-WIN-017, HU-WIN-020 (declaración de `sensitivity`).

**Technical Considerations**: la redacción la define la tool (`redact(ToolResult) -> ToolResult`), con un default que vacía `output`.

**Tests**: reloj falso para retención; test de que un reenvío posterior al settle devuelve el resultado redactado.

**Definition of Done**: DoD común.
