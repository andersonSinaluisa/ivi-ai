# EPIC-05 — Tool System

Fase **P4**. Objetivo: el contrato común de tools y el motor que las ejecuta con límites. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-020 — Contrato de Tool y ToolResult

**Goal**: definir en código el contrato que todas las tools cumplen.

**Description**: en `jarvis-tooling`, los tipos de [Tool Contract](../../03-tools/tool-contract.md): descriptor, operaciones, `ExecutionOutcome`, `VerificationReport`, `ToolResult`, catálogo de errores.

**Acceptance Criteria**
1. Trait `Tool` con `descriptor()`, `validate`, `risk`, `resources`, `execute`, `verify` (async donde aplique).
2. Descriptor con todos los campos obligatorios del documento; `maxMs` obligatorio por tipo.
3. `ToolResult` serializa exactamente al JSON de ejemplo (`schema: "jarvis.tool-result/1"`), con `outcome` derivado de ejecución + verificación según la tabla de [Verification](../../03-tools/verification.md#separación-de-estados).
4. Catálogo de errores como enum con `code`, `category`, `retryable` y mapeo a mensaje del Cloud (`task.rejected` con códigos del Cloud / `task.failed` / `task.cancelled`).
5. `message` de error ≤ 500 caracteres, sin rutas del perfil (sanitizado).

**Dependencies**: HU-WIN-006.

**Technical Considerations**: `async_trait` o trait nativo async; `Invocation { tool, input, principal, limits, cancel, deadline }` como entrada común para Cloud y Local Tool API.

**Tests**: snapshot del JSON de `ToolResult`; tabla ejecución × verificación → outcome.

**Definition of Done**: DoD común.

---

## HU-WIN-021 — Registro de tools y mapeo TaskType

**Goal**: resolver cada TaskType a una tool habilitada y anunciarlo al Cloud.

**Description**: [Tool Registry](../../03-tools/tool-registry.md) y [ADR-015](../../09-decisions/ADR-015-task-type-tool-mapping.md).

**Acceptance Criteria**
1. Registro inmutable construido al arrancar con las tools builtin.
2. `lower(TaskType)` → tool; alias declarados (`OPEN_CHROME` → `open_app {appId: chrome}`).
3. Solo tools listadas en `tools.enabled` se resuelven; el resto → `UNSUPPORTED_TASK_TYPE`.
4. `supportedTaskTypes` del hello = habilitadas + alias.
5. Validación de descriptores al arrancar (nombre único, esquemas compilables, `verification.mode = required` para riesgo base ≥ R2); descriptor inválido → el agente no arranca.
6. `jarvis-agent tools list` muestra nombre, TaskType, riesgo, permisos, habilitada.

**Dependencies**: HU-WIN-020, HU-WIN-002.

**Technical Considerations**: los alias transforman el input antes de la validación.

**Tests**: unit de resolución, alias y deshabilitadas; test que falla si una tool R2 declara `not_applicable`.

**Definition of Done**: DoD común.

---

## HU-WIN-022 — Validación de input y tool `noop`

**Goal**: inputs estrictamente validados y una tool de prueba para el pipeline.

**Description**: etapa 7 del pipeline con JSON Schema y la tool `noop` solo en builds de test (`cfg(feature = "test-tools")`).

**Acceptance Criteria**
1. Validación contra `inputSchema` (draft 2020-12) + validación semántica de la tool; errores → `INVALID_PAYLOAD` con el primer error legible en `message`.
2. Payload > 16 KiB → `INVALID_PAYLOAD`.
3. `validate` devuelve el input normalizado que se usa en el resto del pipeline.
4. `noop { delayMs, outcome, verify }` permite simular éxito, fallo, timeout, verificación fallida y cancelación.
5. `noop` no existe en el binario de release (test que lo comprueba).

**Dependencies**: HU-WIN-020.

**Technical Considerations**: esquemas compilados una vez (`jsonschema` crate) y cacheados en el registro.

**Tests**: tabla de payloads válidos/inválidos; comprobación de features del binario release en CI.

**Definition of Done**: DoD común.

---

## HU-WIN-023 — Executor: scheduler, locks, timeouts y cancelación

**Goal**: ejecutar tools en paralelo sin interferencias y siempre dentro de sus límites.

**Description**: [Concurrency Model](../../01-architecture/concurrency-model.md), [Timeouts](../../01-architecture/execution-pipeline.md#timeouts) y [Cancelación](../../01-architecture/execution-pipeline.md#cancelación).

**Acceptance Criteria**
1. Locks (`DESKTOP_INPUT`, `APP:*`, `PROCESS:*`, `FS:*` lectura/escritura, `COMMAND:*`) adquiridos atómicamente en orden canónico.
2. Máximo `executor.max_concurrent` ejecuciones y cola `queue_capacity`; tiempo en cola cuenta para el timeout efectivo.
3. Timeout efectivo = `min(tool.maxMs, maxDurationSeconds, input/default)`; al vencer → `TOOL_TIMEOUT`.
4. Cancelación `cooperative` (token), `kill` (hook de la tool, p. ej. Job Object) y `none` (se publica `task.progress` "cancellation not supported by tool" y la tarea termina en su estado real).
5. Una tool que no termina tras `maxMs` + 5 s de gracia se abandona (su tarea falla con `TOOL_TIMEOUT`), sus locks **no** se liberan hasta que termine realmente, y se activa `TOOL_STUCK`.
6. Un pánico dentro de una tool se captura: la tarea falla con `SYSTEM_ERROR` y el agente sigue vivo.

**Dependencies**: HU-WIN-020, HU-WIN-022.

**Technical Considerations**: `tokio::time::timeout` + `CancellationToken`; `catch_unwind` alrededor de `spawn_blocking`; métricas `lock_wait_ms`.

**Tests**: unit con `noop` y reloj pausado de Tokio: dos tareas con el mismo lock se serializan; sin locks van en paralelo; timeout; cancelación en cada modo; pánico.

**Definition of Done**: DoD común.
