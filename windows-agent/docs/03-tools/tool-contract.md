# Tool Contract

Contrato conceptual que toda tool cumple (builtin o provider externo). No es código: define **qué** debe declarar y proveer cada tool. Ver [ADR-003](../09-decisions/ADR-003-tool-abstraction.md).

## Descriptor (estático)

| Campo | Tipo | Obligatorio | Descripción |
|---|---|---|---|
| `name` | `snake_case` | ✅ | Único; se corresponde con el TaskType `UPPER_SNAKE_CASE` |
| `version` | semver | ✅ | Cambia el mayor si cambia el esquema de input/output de forma incompatible |
| `description` | texto | ✅ | Para humanos y, en el futuro, para el LLM del Agent Core. Sin detalles internos |
| `riskLevel` | `R0`…`R4` | ✅ | Riesgo **base** ([Risk Model](../04-security/risk-model.md)) |
| `riskModifiers` | lista | — | Reglas que elevan el riesgo según el input (`force=true → R3`) |
| `requiredPermissions` | lista de capabilities | ✅ | Todas deben estar concedidas; pueden depender del input (`commands.run:<id>`) |
| `inputSchema` | JSON Schema 2020-12 | ✅ | `additionalProperties: false`, límites explícitos. Es el contrato publicado (y lo que verá el Agent Core); la validación efectiva la hace `validate` con tipos estrictos (`deny_unknown_fields`) y comprobación semántica, que además normaliza el input |
| `outputSchema` | JSON Schema | ✅ | Salida exitosa |
| `resources` | función del input → locks | ✅ (puede ser vacía) | [Concurrency Model](../01-architecture/concurrency-model.md) |
| `timeouts` | `{ defaultMs, maxMs }` | ✅ | `defaultTimeout` / `maxTimeout` en el resto de la documentación. `maxMs` obligatorio (≤ 24 h); ninguna tool sin límite |
| `cancellation` | `none \| cooperative \| kill` | ✅ | [Cancelación](../01-architecture/execution-pipeline.md#cancelación) |
| `verification` | `{ mode, timeoutMs, pollIntervalMs }` | ✅ | `mode = required` para tools con efectos; `not_applicable` solo para tools de observación pura |
| `sensitivity` | `none \| output \| input_and_output` | ✅ | Si el resultado se redacta tras *settled* y cómo se loguea |
| `maxOutputBytes` | entero ≤ 24 576 | ✅ | Cabe en el `result` del Cloud (32 KiB) con el envoltorio |
| `requiresInteractiveSession` | bool | ✅ | Si falla con sesión bloqueada/desconectada |
| `idempotentEffect` | bool | ✅ | Informativo (p. ej. `close_app` sobre una app ya cerrada es no-op). No cambia la regla de una ejecución por `taskId` |

## Operaciones

| Operación | Contrato | Errores típicos |
|---|---|---|
| `validate(input)` | Pura (sin efectos). Valida esquema y semántica local (`appId` existe en el catálogo, ruta dentro de allowed roots). Devuelve el input **normalizado** (p. ej. ruta canónica) | `INVALID_PAYLOAD`, `APP_NOT_FOUND`, `PATH_NOT_ALLOWED`, `COMMAND_NOT_FOUND` |
| `risk(input)` | Pura. Riesgo efectivo = máx(base, modificadores) | — |
| `resources(input)` | Pura. Locks requeridos | — |
| `execute(input, ctx)` | Produce el efecto usando solo interfaces de Platform. `ctx` aporta `executionId`, deadline, token de cancelación, logger con spans. Devuelve `ExecutionOutcome` con datos para verificar (p. ej. PID, instante de lanzamiento) | `EXECUTION_FAILED`, `APP_ELEVATED`, `ACCESS_DENIED`, `SESSION_UNAVAILABLE`, `TOOL_TIMEOUT`, `CANCELLED` |
| `verify(outcome, observer)` | Solo observa (sin efectos). Sondea hasta cumplir la condición o agotar su timeout. Devuelve `VerificationReport` | `VERIFICATION_FAILED`, `VERIFICATION_INCONCLUSIVE` |

Reglas:
- `execute` **no** decide permisos ni reintenta por su cuenta salvo reintentos internos declarados (p. ej. esperar a que un proceso cree su ventana).
- `verify` **no** puede producir efectos (ni siquiera "reintentar la acción"): si la verificación falla, la tarea falla y el reintento es decisión del solicitante.
- Una tool nunca escribe en logs contenido de archivos, salida de comandos, títulos sin redactar ni argumentos sensibles.

## ToolResult

Formato estándar, usado en `task.completed.data.result` y en la futura API local.

```json
{
  "schema": "jarvis.tool-result/1",
  "tool": "close_app",
  "toolVersion": "1.0.0",
  "executionId": "0192a6f4-9250-7d11-8000-00000000e002",
  "outcome": "SUCCEEDED",
  "startedAt": "2026-09-19T15:10:00.000Z",
  "finishedAt": "2026-09-19T15:10:03.120Z",
  "durationMs": 3120,
  "execution": { "status": "SUCCEEDED", "durationMs": 45, "details": { "method": "wm_close", "windowsSignaled": 2 } },
  "verification": {
    "status": "PASSED",
    "method": "process_absent",
    "durationMs": 3075,
    "checks": [ { "name": "no_process_for_app", "passed": true, "observed": { "remaining": 0 } } ]
  },
  "output": { "appId": "spotify", "closedProcesses": 3, "forced": false },
  "truncated": false,
  "error": null,
  "metadata": { "riskLevel": "R2", "decision": "APPROVED", "invokedBy": "cloud" }
}
```

| Campo | Descripción |
|---|---|
| `outcome` | `SUCCEEDED` \| `FAILED` \| `CANCELLED`. `SUCCEEDED` **solo** si `execution.status = SUCCEEDED` y `verification.status ∈ {PASSED, NOT_APPLICABLE}` |
| `execution.status` | `SUCCEEDED` \| `FAILED` \| `TIMED_OUT` \| `CANCELLED` |
| `verification.status` | `PASSED` \| `FAILED` \| `INCONCLUSIVE` \| `NOT_APPLICABLE` \| `SKIPPED` (no se verificó porque la ejecución falló) |
| `verification.checks` | Condiciones evaluadas con lo observado (sin datos sensibles) |
| `output` | Validado contra `outputSchema`; ≤ `maxOutputBytes` |
| `truncated` | `true` si la salida se recortó para cumplir límites |
| `error` | `null` o `{ code, message, retryable, category }` |
| `metadata` | `riskLevel`, `decision`, `invokedBy` (`cloud` \| `local`). **Sin** hostname, usuario, rutas del sistema |
| Timestamps | UTC ISO-8601 con milisegundos |

## Errores

Cada error tiene `code` estable, `category`, `retryable` y un `message` corto sin datos sensibles. **Nunca** stack traces, códigos HRESULT crudos con rutas, nombres de usuario ni contenido.

| Código | Categoría | Etapa | Mensaje al Cloud | `retryable` |
|---|---|---|---|---|
| `INVALID_ENVELOPE` | `VALIDATION_ERROR` | Deserialización / envelope (etapas 1–2) | Ninguno (solo log `WARN`); si el `taskId` es legible, `task.rejected INVALID_PAYLOAD` | — |
| `SIGNATURE_INVALID` | `PERMISSION_DENIED` | Autenticación (etapa 3, P8) | `task.rejected NOT_PERMITTED` | false |
| `INVALID_PAYLOAD` | `VALIDATION_ERROR` | Validación | `task.rejected` (código del Cloud) | false |
| `UNSUPPORTED_TASK_TYPE` | `TOOL_NOT_FOUND` | Resolución | `task.rejected` (código del Cloud) | false |
| `EXPIRED_ON_ARRIVAL` | `VALIDATION_ERROR` | Expiración | `task.rejected` (código del Cloud) | false |
| `NOT_PERMITTED` | `PERMISSION_DENIED` | Política / firma | `task.rejected` (código del Cloud) | false |
| `APPROVAL_DENIED` | `APPROVAL_REQUIRED` | Aprobación | `task.cancelled` reason | false |
| `APPROVAL_EXPIRED` | `APPROVAL_REQUIRED` | Aprobación | `task.cancelled` reason | true |
| `APP_NOT_FOUND` | `APP_NOT_FOUND` | Validación (no está en el catálogo) → `task.rejected INVALID_PAYLOAD`; ejecución (ejecutable desaparecido) → `task.failed APP_NOT_FOUND` | ver columna | false |
| `APP_ELEVATED` | `EXECUTION_FAILED` | Ejecución (proceso de integridad alta, no se puede cerrar/observar) | `task.failed` | false |
| `ACCESS_DENIED` | `EXECUTION_FAILED` | Ejecución (ACL de Windows) | `task.failed` | false |
| `SESSION_UNAVAILABLE` | `EXECUTION_FAILED` | Ejecución (sesión bloqueada/desconectada y la tool requiere sesión interactiva) | `task.failed` | true |
| `EXECUTION_FAILED` | `EXECUTION_FAILED` | Ejecución (genérico) | `task.failed` | depende |
| `VERIFICATION_FAILED` | `VERIFICATION_FAILED` | Verificación: condición no se cumplió | `task.failed` | true |
| `VERIFICATION_INCONCLUSIVE` | `VERIFICATION_FAILED` | Verificación: no se pudo observar | `task.failed` | true |
| `TOOL_TIMEOUT` | `TIMEOUT` | Ejecución o verificación | `task.failed` | true |
| `OUTPUT_TOO_LARGE` | `EXECUTION_FAILED` | Salida no truncable | `task.failed` | false |
| `PATH_NOT_ALLOWED`, `FILE_NOT_FOUND`, `FILE_TOO_LARGE`, `BINARY_FILE` | `VALIDATION_ERROR` / `EXECUTION_FAILED` | Filesystem (P7) | `task.rejected INVALID_PAYLOAD` o `task.failed` | false |
| `COMMAND_NOT_FOUND`, `COMMAND_FAILED` | `VALIDATION_ERROR` / `EXECUTION_FAILED` | Comandos (P7) | `task.rejected INVALID_PAYLOAD` / `task.failed` | false / depende |
| `ELEMENT_NOT_FOUND`, `ELEMENT_AMBIGUOUS` | `EXECUTION_FAILED` | UI Automation (P9) | `task.failed` | true / false |
| `CANCELLED` | `CANCELLED` | Cancelación | `task.cancelled USER_REQUEST` | — |
| `EXECUTION_INTERRUPTED` | `SYSTEM_ERROR` | Recuperación (crash durante `Running`) | `task.failed` ([C6](../00-overview/cloud-compatibility.md)) | true |
| `STATE_RESET` | `SYSTEM_ERROR` | Tarea anterior a un reinicio de la BD local | `task.rejected EXPIRED_ON_ARRIVAL` | false |
| `SYSTEM_ERROR` | `SYSTEM_ERROR` | Fallo interno inesperado | `task.failed` (mensaje genérico) | true |

`task.rejected` usa siempre los códigos del catálogo del Cloud (`INVALID_PAYLOAD`, `UNSUPPORTED_TASK_TYPE`, `EXPIRED_ON_ARRIVAL`, `NOT_PERMITTED`); el código fino del agente va en `message` y en los logs locales ([C5](../00-overview/cloud-compatibility.md)).
