# EPIC-05 — Task Management

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [Task State Machine](../../02-domain/task-state-machine.md), [ADR-006](../../08-decisions/ADR-006-task-delivery-semantics.md).

---

## HU-024 — Dominio JarvisTask y máquina de estados

- **Objetivo:** encapsular todas las reglas de estado de tareas en el dominio.
- **Descripción:** entidad `JarvisTask` con métodos `Dispatch(now)`, `RevertToPending(reason)`, `Acknowledge(now)`, `Reject(error)`, `Start(now)`, `ReportProgress(...)`, `RequireApproval(reason)`, `Complete(result, now)`, `Fail(error, now)`, `ConfirmCancelled(reason, now)`, `RequestCancel(now)`, `Expire(now)`, `FailByTimeout(now)`, `FailDeviceLost(now)`. Cada método aplica la regla de avance monótono y devuelve `Applied`/`Ignored(reason)`; registra eventos de dominio incluidos los intermedios omitidos.
- **Criterios de aceptación:**
  1. Todas las transiciones T1-T17 cubiertas por tests.
  2. Todas las transiciones inválidas de la tabla devuelven `Ignored` o error de dominio según corresponda.
  3. Aplicar el mismo mensaje 2 veces → segundo `Ignored`, sin eventos nuevos.
  4. Saltos de estado rellenan solo lo que el mensaje implica: desde `Dispatched`, cualquier mensaje posterior al ack fija `delivered_at` y emite `TASK_DELIVERED`; nunca se inventa `TASK_STARTED` (ver consideración técnica).
  5. Invariantes: `Completed` requiere `result`; `Failed`/`Cancelled`/`Expired` requieren `error_code`; `completed_at` en terminales.
  6. Validación de `TaskType`, tamaño de `payload` (16 KiB) y `result` (32 KiB → `Failed RESULT_TOO_LARGE`).
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** regla de relleno: al pasar desde `Dispatched` directamente a `Running` o a un terminal, se fija `delivered_at` y se emite `TASK_DELIVERED`; al pasar a terminal desde `Delivered`/`Dispatched` **no** se inventa `TASK_STARTED` (no hay evidencia de inicio). Test de propiedades (secuencias aleatorias de mensajes) recomendado: el estado final nunca retrocede de rango.
- **Pruebas esperadas:** unit tests exhaustivos en `Jarvis.Domain.Tests`.
- **Definition of Done:** DoD común + la regla de relleno queda también reflejada en [Task State Machine](../../02-domain/task-state-machine.md) si el implementador la refina.

---

## HU-025 — Gestión de task grants

- **Objetivo:** endpoints de grants según [REST API](../../03-protocols/rest-api.md#grants).
- **Descripción:** tabla `device_task_grants`; `GET/POST/DELETE /api/v1/devices/{id}/grants`; eventos `GRANT_CREATED`/`GRANT_DELETED`.
- **Criterios de aceptación:**
  1. Origen y destino deben ser del owner (404 si no).
  2. `taskType = "*"` o inválido → 400; origen = destino → 422 `SAME_DEVICE`; duplicado → 409.
  3. No se pueden crear grants sobre devices revocados (409 `DEVICE_REVOKED`).
  4. `DELETE` idempotente.
- **Dependencias:** HU-012.
- **Consideraciones técnicas:** consulta de autorización `EXISTS (source, target, type)` usando el índice único.
- **Pruebas esperadas:** integración con dos owners.
- **Definition of Done:** DoD común.

---

## HU-026 — Crear tarea por REST

- **Objetivo:** `POST /api/v1/tasks` según [REST API](../../03-protocols/rest-api.md#post-apiv1tasks) y [Authorization](../../05-security/authorization.md#reglas-de-post-tasks--taskcreate).
- **Descripción:** caso de uso `CreateTask` compartido por REST y MQTT; tabla `tasks` con índices; idempotencia con `request_hash`; `TASK_CREATED` / `TASK_REJECTED`; señal en memoria al dispatcher.
- **Criterios de aceptación:**
  1. User crea tarea para su device → 201 `Pending`.
  2. Device sin grant → 403 `TASK_NOT_GRANTED` + `TASK_REJECTED`; con grant → 201 y `source_device_id` fijado.
  3. Device sin `Idempotency-Key` → 400 `IDEMPOTENCY_KEY_REQUIRED`.
  4. Misma key y mismo contenido → 200 con la misma tarea y `Idempotent-Replayed: true`; distinto contenido → 422.
  5. Target deshabilitado → 409; tipo no soportado → 422; payload > 16 KiB → 413.
  6. `correlationId` provisto se respeta; si no, se genera; aparece en `TASK_CREATED` y logs.
  7. Dos requests concurrentes con la misma key crean **una** tarea (índice único + manejo de violación).
- **Dependencias:** HU-024, HU-025, HU-017.
- **Consideraciones técnicas:** `request_hash` = SHA-256 de JSON canónico `(targetDeviceId, type, payload, expiresInSeconds, maxDurationSeconds)`.
- **Pruebas esperadas:** integración de todos los criterios, incluida concurrencia.
- **Definition of Done:** DoD común.

---

## HU-027 — Crear tarea por MQTT

- **Objetivo:** `task.create` en `up/requests` ([MQTT Protocol §4.1-4.2](../../03-protocols/mqtt-protocol.md#41-uprequests--taskcreate)).
- **Descripción:** handler que valida el mensaje, llama `CreateTask` con principal Device y responde `request.accepted` (`duplicate` según idempotencia) o `request.rejected` con `code`; `causationId` = id del request; rate limit 30/min por device.
- **Criterios de aceptación:**
  1. Mismas reglas y códigos que HU-026.
  2. Reenvío del mismo mensaje → `request.accepted` con `duplicate: true` y mismo `taskId`.
  3. `ts` fuera de ventana → `STALE_MESSAGE`.
  4. Exceso de rate → `RATE_LIMITED`.
- **Dependencias:** HU-026, HU-022, HU-023.
- **Consideraciones técnicas:** la respuesta se publica después del commit.
- **Pruebas esperadas:** integración con un cliente MQTTnet autenticado como device.
- **Definition of Done:** DoD común.

---

## HU-028 — Dispatcher de tareas

- **Objetivo:** publicar tareas `Pending` a targets `Online`.
- **Descripción:** hosted service `TaskDispatcher` despertado por señal (creación, hello) o cada 5 s; claim por lotes con `FOR UPDATE SKIP LOCKED` ([Persistence Strategy](../../04-data/persistence-strategy.md#acceso-con-ef-core)); transición a `Dispatched`, `TASK_DISPATCHED {attempt}`; commit; publicación `task.dispatch`. También reenvía `task.cancel` cada 60 s a tareas con `cancel_requested_at` no nulo y estado no terminal.
- **Criterios de aceptación:**
  1. Tarea para target Online se publica en < 1 s tras la creación.
  2. Tarea para target Offline permanece `Pending` sin intentos.
  3. Al conectarse el target (hello), sus `Pending` se despachan.
  4. Solo tareas con `expires_at > now`.
  5. Dos dispatchers concurrentes (test) nunca despachan la misma tarea en el mismo ciclo.
  6. Heartbeat del servicio visible en readiness.
- **Dependencias:** HU-026, HU-023, HU-035.
- **Consideraciones técnicas:** orden FIFO por `created_at` por target. Lote máximo 50.
- **Pruebas esperadas:** integración con Mosquitto y un device de prueba.
- **Definition of Done:** DoD común.

---

## HU-029 — Procesar actualizaciones de tarea del device

- **Objetivo:** aplicar `up/tasks` a la máquina de estados ([MQTT Protocol §4.4-4.5](../../03-protocols/mqtt-protocol.md#44-uptasks--ciclo-de-vida)).
- **Descripción:** handlers para `task.ack`, `task.rejected`, `task.started`, `task.progress`, `task.approval_required`, `task.completed`, `task.failed`, `task.cancelled`; verificación de que el emisor es el target; actualización condicional con reintento ante conflicto; `task.update.ack` para estados terminales (incluidos tardíos); eventos correspondientes.
- **Criterios de aceptación:**
  1. Secuencia normal ack → started → completed produce `Delivered` → `Running` → `Completed` y 3 eventos.
  2. Duplicados no generan eventos adicionales.
  3. Mensaje de un device que no es el target → `SECURITY_VIOLATION`, sin cambios.
  4. Terminal tardío sobre tarea terminal → `task.update.ack` con el estado real del Cloud + log `late_terminal_update`.
  5. `task.progress` actualiza `progress_*` y `last_update_at` sin evento.
  6. `result` > 32 KiB → `Failed RESULT_TOO_LARGE`.
- **Dependencias:** HU-028, HU-022.
- **Consideraciones técnicas:** procesamiento en orden por device (HU-021).
- **Pruebas esperadas:** integración con MQTT real; duplicación y desorden simulados.
- **Definition of Done:** DoD común.

---

## HU-030 — ACK timeout y redelivery

- **Objetivo:** at-least-once cuando se pierde un dispatch o un ACK.
- **Descripción:** `DeliveryTimeoutSweeper` cada 10 s: `Dispatched` con `dispatched_at < now - 30 s` → `Pending` (`ack_timeout`) si `attempt_count < max_attempts`, si no → `Failed DELIVERY_FAILED`.
- **Criterios de aceptación:**
  1. Device que ignora el dispatch recibe hasta 5 intentos y la tarea termina `Failed DELIVERY_FAILED`.
  2. Device que ack en el intento 2 → `Delivered` con `attempt_count = 2`.
  3. El device de prueba que deduplica no ejecuta dos veces (verificado por su contador).
- **Dependencias:** HU-028.
- **Consideraciones técnicas:** `TimeProvider` falso en tests.
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-031 — Expiración y timeout de ejecución

- **Objetivo:** que ninguna tarea quede abierta para siempre.
- **Descripción:** `TaskExpirySweeper` cada 15 s: `Pending` vencidas → `Expired`; `Dispatched` vencidas + 30 s → `Expired`; `Delivered`/`Running`/`WaitingForApproval` con `delivered_at + max_duration_seconds` (o 24 h) vencido → publicar `task.cancel` y `Failed EXECUTION_TIMEOUT`.
- **Criterios de aceptación:**
  1. Cada caso transiciona con su evento y código.
  2. Una tarea con `max_duration_seconds` nulo expira a las 24 h de `delivered_at`.
  3. El origen recibe `task.update` (vía HU-034).
- **Dependencias:** HU-029.
- **Consideraciones técnicas:** actualizaciones condicionales por lote.
- **Pruebas esperadas:** integración con reloj falso.
- **Definition of Done:** DoD común.

---

## HU-032 — Cancelar tarea

- **Objetivo:** `POST /api/v1/tasks/{id}/cancel` y `task.cancel_request` por MQTT ([Data Flow §6](../../01-architecture/data-flow.md#6-cancelación)).
- **Descripción:** `Pending` → `Cancelled` (200); estados activos → `cancel_requested_at`, `TASK_CANCEL_REQUESTED`, `task.cancel` al target (202); terminal → 409.
- **Criterios de aceptación:**
  1. Solo owner o device origen pueden cancelar; target → 403 `FORBIDDEN` (REST) / `FORBIDDEN` (MQTT).
  2. Llamadas repetidas no duplican eventos.
  3. El device responde `task.cancelled` → `Cancelled`; si responde `task.completed` → `Completed` prevalece.
  4. El dispatcher reenvía `task.cancel` cada 60 s mientras siga pendiente.
- **Dependencias:** HU-029.
- **Consideraciones técnicas:** —
- **Pruebas esperadas:** integración de ambos canales.
- **Definition of Done:** DoD común.

---

## HU-033 — Consultar tareas por REST

- **Objetivo:** `GET /api/v1/tasks` y `GET /api/v1/tasks/{id}`.
- **Descripción:** filtros, paginación por cursor, representación completa en detalle y liviana en listado; alcance por principal ([Authorization](../../05-security/authorization.md#matriz-rest)).
- **Criterios de aceptación:**
  1. User ve todas las tareas de su owner; device solo donde es origen o target.
  2. Filtros por `status` múltiple, `correlationId`, fechas.
  3. Recurso ajeno → 404.
- **Dependencias:** HU-026.
- **Consideraciones técnicas:** índices `ix_tasks_owner_created`, `ix_tasks_source_created`.
- **Pruebas esperadas:** integración con dos owners y dos devices.
- **Definition of Done:** DoD común.

---

## HU-034 — Notificaciones al device origen y `task.get`

- **Objetivo:** que el ESP32 reciba el resultado de lo que pidió.
- **Descripción:** en cada cambio de estado de una tarea con `source_device_id`, publicar `task.update` en `down/notifications` del origen (después del commit); handler `task.get` que responde `task.update` o `request.rejected NOT_FOUND`.
- **Criterios de aceptación:**
  1. El origen recibe `task.update` para `Dispatched`, `Delivered`, `Running`, `WaitingForApproval` y terminales, con `result`/`error` en terminales.
  2. `task.get` de una tarea ajena → `NOT_FOUND`.
  3. Si el origen está offline, no se reintenta (best effort) y `task.get` al reconectar devuelve el estado final.
- **Dependencias:** HU-027, HU-029.
- **Consideraciones técnicas:** reutiliza el mecanismo de eventos de dominio → publicador post-commit.
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-057 — Aprobación remota de tareas (Phase 3, implementada)

- **Objetivo:** permitir que el owner apruebe o deniegue en remoto una tarea que el device dejó en `WaitingForApproval`.
- **Descripción:** `POST /api/v1/tasks/{id}/approval` con `{ decision, nonce, decidedAt, signature }`; mensaje `down/tasks` `task.approval_decision`; tabla `task_approvals`. La decisión la **firma el cliente del owner**, no el Cloud ([ADR-012](../../08-decisions/ADR-012-end-to-end-task-signing.md)).
- **Criterios de aceptación:**
  1. Una decisión firmada sobre una tarea en `WaitingForApproval` responde `202` y llega al device con `decision`, `nonce`, `decidedAt` y `signature` **idénticos** a lo enviado.
  2. Una decisión sin firma, con firma mal formada, con nonce inválido o fuera de la ventana de ±5 minutos se rechaza con `400` y **no** se transmite.
  3. Una segunda decisión sobre la misma tarea responde `409 APPROVAL_ALREADY_DECIDED`; un nonce ya usado por el owner, `409 NONCE_REUSED`.
  4. Una tarea que no está esperando aprobación responde `409 INVALID_STATE_TRANSITION`.
  5. Se registra `APPROVAL_DECISION_SENT` (con `signatureKeyId`). El `APPROVAL_GRANTED`/`APPROVAL_DENIED` solo aparece si el device la aplica.
  6. `202` significa "transmitida", nunca "aprobada".
- **Dependencias:** HU-029, HU-042, ADR-012.
- **Consideraciones técnicas:** una aprobación remota es una capacidad sensible: si el Cloud pudiera aprobar, comprometerlo bastaría para aprobar cualquier cosa. Por eso el Cloud no verifica ni firma, y el device revalida firma, nonce y frescura antes de aplicarla.
- **Pruebas:** `SigningTests` (integración, 7 casos de aprobación) y `TaskSigningTests` (dominio); de extremo a extremo contra el agente real en [WIN-E2E-16](../../../../windows-agent/docs/11-testing/integration-scenarios.md).
- **Definition of Done:** cumplida.
