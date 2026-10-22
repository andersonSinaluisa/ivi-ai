# EPIC-08 — Verification

Fase **P5**. Objetivo: que ninguna tarea se reporte completada sin evidencia observable. Diseño en [Verification](../../03-tools/verification.md). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-035 — Verification Engine

**Goal**: una fase de verificación separada, con timeout y resultado explícito.

**Description**: motor en `jarvis-tooling` que ejecuta `verify` de la tool tras una ejecución exitosa.

**Acceptance Criteria**
1. Estado local `Verifying` persistido antes de verificar.
2. Sondeo cada `pollIntervalMs` hasta cumplir o agotar `verification.timeoutMs`, dentro del timeout efectivo de la tarea.
3. Resultado `PASSED` / `FAILED` / `INCONCLUSIVE` / `NOT_APPLICABLE` / `SKIPPED` con `checks[]` y `durationMs`.
4. `INCONCLUSIVE` y `FAILED` → `outcome FAILED`; la verificación nunca re-ejecuta la acción.
5. El Observer que recibe `verify` es de solo lectura (tipo sin métodos con efectos).
6. `task.cancel` durante `Verifying` no interrumpe la verificación (solo observa); se reporta el estado terminal real, como indica [Cancelación](../../01-architecture/execution-pipeline.md#cancelación).

**Dependencies**: HU-WIN-020, HU-WIN-023.

**Technical Considerations**: el Observer se construye sobre `ProcessApi`/`WindowApi` exponiendo solo lecturas.

**Tests**: unit con `noop` configurado para pasar tras N sondeos, no pasar nunca, y no poder observar.

**Definition of Done**: DoD común.

---

## HU-WIN-036 — Métodos `process_and_window` y `process_absent`

**Goal**: verificar con precisión la apertura y el cierre de apps reales.

**Description**: métodos de la tabla de [Verification](../../03-tools/verification.md#métodos).

**Acceptance Criteria**
1. `process_and_window`: proceso de la app vivo (el PID lanzado o, para instancia única/launcher, un proceso de la app creado después de `launchedAt` o con ventana nueva) **y** ventana top-level visible no cloaked.
2. Si el PID lanzado sale con código ≠ 0 antes de cumplirse → `FAILED` con `exitCode` en el check.
3. `process_absent`: ningún proceso de la app en la sesión durante dos sondeos consecutivos (evita falsos positivos por relanzamiento).
4. Timeouts por entrada del catálogo (`verify_timeout_ms`), default 10 s / 5 s.
5. Checks con nombres estables: `process_running`, `top_level_window_visible`, `no_process_for_app`.

**Dependencies**: HU-WIN-035, HU-WIN-028, HU-WIN-029.

**Technical Considerations**: una sola enumeración de procesos y ventanas por sondeo.

**Tests**: unit con fakes (launcher, instancia única, cloaked, relanzamiento); integración Windows con apps de prueba.

**Definition of Done**: DoD común.

---

## HU-WIN-037 — Verificación reflejada en resultados y mensajes

**Goal**: que el Cloud y el usuario vean exactamente qué se comprobó.

**Description**: mapeo `ToolResult` → `task.completed` / `task.failed` de [Task Protocol](../../02-cloud-integration/task-protocol.md).

**Acceptance Criteria**
1. `task.completed` solo con `outcome = SUCCEEDED`; `result` = `ToolResult` completo con `verification`.
2. `task.failed` con `error.code` `VERIFICATION_FAILED` o `VERIFICATION_INCONCLUSIVE`, `message` que resume el check fallido, `retryable: true`.
3. El `ToolResult` completo de las tareas fallidas se guarda localmente y aparece en `diagnostics` (HU-WIN-052).
4. Tabla de CI que recorre todas las tools registradas y comprueba que las de riesgo ≥ R2 tienen verificación `required` (refuerzo de HU-WIN-021).

**Dependencies**: HU-WIN-035, HU-WIN-039.

**Technical Considerations**: el `message` no incluye títulos ni rutas.

**Tests**: contract test del JSON de `task.completed` con `verification.status = "PASSED"`; integración con el Cloud: `GET /api/v1/tasks/{id}` muestra `result.verification`.

**Definition of Done**: DoD común.
