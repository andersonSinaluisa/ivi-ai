# EPIC-09 — Task Execution

Fases **P3–P5**. Objetivo: el pipeline completo de 17 etapas de [Execution Pipeline](../../01-architecture/execution-pipeline.md) que une Cloud Link, estado, política, executor y verificación. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-038 — Intake de tareas (etapas 1–7 y 11)

**Goal**: aceptar o rechazar cada `task.dispatch` con el mensaje correcto y de forma durable.

**Description**: etapas 1–7 y 11 del pipeline. En P3, antes de existir tools, todo tipo resuelve a `UNSUPPORTED_TASK_TYPE`; la política (8–10) se conecta en HU-WIN-026/027.

**Acceptance Criteria**
1. Orden de etapas: deserialize → envelope → autenticación de canal → idempotencia → expiración (reloj corregido) → resolución → validación → persistencia + `task.ack`.
2. Rechazos con códigos del Cloud: `EXPIRED_ON_ARRIVAL`, `UNSUPPORTED_TASK_TYPE`, `INVALID_PAYLOAD`, `NOT_PERMITTED`; el código fino del agente en `message`.
3. Tarea conocida ya vencida → se responde su estado (idempotencia antes que expiración).
4. Guardia `STATE_RESET`: `createdAt < db_created_at` → `task.rejected EXPIRED_ON_ARRIVAL` con motivo `STATE_RESET`.
5. Si la BD falla al persistir → no hay `task.ack` (el Cloud reintentará).
6. `cancelRequested = true` en un dispatch nuevo → tratada como cancelada antes de iniciar.

**Dependencies**: HU-WIN-010, HU-WIN-016, HU-WIN-017.

**Technical Considerations**: el pipeline trabaja con `Invocation` (HU-WIN-020) para reutilizarse desde la Local Tool API.

**Tests**: tabla de dispatches → mensaje esperado; integración con el Cloud: tipo desconocido → `Rejected` en el Cloud.

**Definition of Done**: DoD común + [WIN-E2E-02](../../11-testing/integration-scenarios.md#win-e2e-02-tarea-inválida) y [WIN-E2E-06](../../11-testing/integration-scenarios.md#win-e2e-06-tool-inexistente) verdes.

---

## HU-WIN-039 — Ejecución y cierre (etapas 12–17)

**Goal**: llevar una tarea aceptada hasta su estado terminal asentado.

**Description**: etapas 12–17: schedule, `task.started`, execute, verify, persistir resultado, publicar, settle.

**Acceptance Criteria**
1. `Running` y `executionId` (UUIDv7) persistidos **antes** de llamar a `execute`, con `task.started` en el outbox.
2. Resultado + mensaje terminal en una transacción; publicación desde el outbox.
3. `busy` en presencia refleja si hay ejecuciones en curso.
4. `task.update.ack` → *settled* + redacción (HU-WIN-019).
5. Todos los estados locales de [Estados locales](../../01-architecture/execution-pipeline.md#estados-locales) y su correspondencia con el Cloud.
6. Logs con span `taskId`/`executionId`/`correlationId`/`tool`/`durationMs`/`result`.

**Dependencies**: HU-WIN-038, HU-WIN-023, HU-WIN-035.

**Technical Considerations**: la máquina de estados local es un tipo con transiciones válidas explícitas (transición inválida = bug → `SYSTEM_ERROR`).

**Tests**: integración con `noop`: éxito, fallo de ejecución, timeout, verificación fallida; cada caso comprobado en el Cloud.

**Definition of Done**: DoD común + [WIN-E2E-07](../../11-testing/integration-scenarios.md#win-e2e-07-ejecución-fallida) y [WIN-E2E-12](../../11-testing/integration-scenarios.md#win-e2e-12-timeout) verdes.

---

## HU-WIN-040 — Cancelación de tareas

**Goal**: respetar `task.cancel` del Cloud en cada estado.

**Description**: tabla de [Cancelación](../../01-architecture/execution-pipeline.md#cancelación).

**Acceptance Criteria**
1. `AwaitingApproval` / `Accepted` → cancelada sin ejecutar → `task.cancelled USER_REQUEST`.
2. `Running` con tool `cooperative` o `kill` → se cancela → `task.cancelled`; si la tool termina antes de poder cancelarse, se reporta su estado terminal real.
3. `Running` con tool `none` → `task.progress` "cancellation not supported by tool" y estado terminal real.
4. Terminal → se reenvía el terminal existente.
5. `task.cancel` repetido → idempotente.

**Dependencies**: HU-WIN-039.

**Technical Considerations**: el `CancellationToken` de cada ejecución se guarda en un mapa por `taskId`.

**Tests**: integración con `noop` lento en cada modo de cancelación; `close_app` durante el período de gracia.

**Definition of Done**: DoD común + [WIN-E2E-11](../../11-testing/integration-scenarios.md#win-e2e-11-cancelación) verde.

---

## HU-WIN-041 — Aprobación dentro del pipeline

**Goal**: integrar las aprobaciones (HU-WIN-027) en el flujo de mensajes del Cloud.

**Description**: etapa 10 del pipeline.

**Acceptance Criteria**
1. `REQUIRES_APPROVAL` → `task.ack` + `task.approval_required { reason }` (el Cloud pasa a `WaitingForApproval`).
2. `Granted` → re-evaluación → cola → `task.started`.
3. `Denied` → `task.cancelled APPROVAL_DENIED`; `Expired` → `task.cancelled APPROVAL_EXPIRED` ([C9](../../00-overview/cloud-compatibility.md)).
4. El timeout de ejecución empieza al iniciar la ejecución, no al pedir aprobación.

**Dependencies**: HU-WIN-027, HU-WIN-039.

**Technical Considerations**: verificado en el Cloud implementado: `TaskLifecycle` guarda cualquier `reason` de `task.cancelled` como `error_code`, así que `APPROVAL_EXPIRED` funciona sin cambios de código; solo falta documentarlo en el Cloud (C9).

**Tests**: integración con el Cloud real para los tres desenlaces.

**Definition of Done**: DoD común.

---

## HU-WIN-042 — Vertical slice con Jarvis Cloud

**Goal**: demostrar de extremo a extremo el objetivo del MVP.

**Description**: la [Demostración obligatoria](../../00-overview/mvp.md#demostración-obligatoria).

**Acceptance Criteria**
1. `POST /api/v1/tasks` con `OPEN_APP {"appId":"chrome"}` → Chrome abierto → tarea `Completed` con `result.verification.status = "PASSED"` y `executionId`.
2. Lo mismo para `IS_APP_RUNNING`, `LIST_RUNNING_APPS`, `GET_ACTIVE_WINDOW` y `CLOSE_APP`.
3. El botón del ESP32 Simulator (`OPEN_CHROME`) abre Chrome.
4. Script reproducible (`scripts/e2e-vertical-slice.ps1`) que ejecuta el flujo contra un Cloud local (`api/` con docker compose).

**Dependencies**: HU-WIN-033, HU-WIN-034, HU-WIN-030, HU-WIN-031, HU-WIN-032, HU-WIN-037, HU-WIN-014.

**Technical Considerations**: requiere un runner Windows con sesión interactiva (máquina de desarrollo o runner self-hosted con auto-logon); los runners hospedados de GitHub no tienen escritorio interactivo fiable para ventanas.

**Tests**: el propio script; grabación del resultado en el PR.

**Definition of Done**: DoD común + [WIN-E2E-01](../../11-testing/integration-scenarios.md#win-e2e-01-tarea-válida-open_app) y [WIN-E2E-14](../../11-testing/integration-scenarios.md#win-e2e-14-alias-open_chrome-desde-el-esp32-simulator) verdes.
