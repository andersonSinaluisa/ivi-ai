# Task Protocol

Cómo viaja una invocación de tool dentro de los mensajes del Cloud. Mapeo TaskType ↔ tool en [ADR-015](../09-decisions/ADR-015-task-type-tool-mapping.md) y [Tool Registry](../03-tools/tool-registry.md).

## Regla de mapeo

| Mensaje del Cloud | Campo | Significado para el agente |
|---|---|---|
| `task.dispatch.data.type` | `OPEN_APP` | Tool `open_app` (`lower(type)`), salvo alias declarados |
| `task.dispatch.data.payload` | objeto ≤ 16 KiB | **Input de la tool**, validado por su esquema |
| `task.dispatch.data.requestedBy` | `{ type, id }` | Principal remoto para la política |
| `task.dispatch.data.expiresAt` | fecha | Límite para **empezar**; vencida → `EXPIRED_ON_ARRIVAL` |
| `task.dispatch.data.maxDurationSeconds` | entero opcional | Tope del timeout efectivo |
| `task.dispatch.data.cancelRequested` | bool | Si es `true` en un dispatch nuevo, se trata como cancelada antes de iniciar |
| `task.dispatch.data.signature` | `null` en v1 | Verificada cuando exista (P8) |
| `task.completed.data.result` | objeto ≤ 32 KiB | **`ToolResult`** ([Tool Contract](../03-tools/tool-contract.md#toolresult)) |
| `task.failed.data.error` | `{ code, message, retryable }` | Error de ejecución o verificación ([catálogo](../03-tools/tool-contract.md#errores)) |
| `task.rejected.data.error` | `{ code, message, retryable: false }` | Rechazo previo a la ejecución con códigos del Cloud |

## Ejemplos completos

### Entrada: `task.dispatch`

```json
{
  "v": 1,
  "id": "0192a6f4-9100-7c3a-9d2e-000000000020",
  "type": "task.dispatch",
  "ts": "2026-09-19T15:04:05.400Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "data": {
    "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001",
    "type": "OPEN_APP",
    "payload": { "appId": "chrome" },
    "attempt": 1,
    "createdAt": "2026-09-19T15:04:05.250Z",
    "expiresAt": "2026-09-19T15:09:05.250Z",
    "maxDurationSeconds": 60,
    "requestedBy": { "type": "Device", "id": "0192a6f0-1111-7aaa-8bbb-000000000001" },
    "cancelRequested": false,
    "signature": null
  }
}
```

### `task.ack` (tras persistir) y `task.started`

```json
{ "v": 1, "id": "0192a6f4-9200-7c3a-9d2e-000000000030", "type": "task.ack", "ts": "2026-09-19T15:04:05.460Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002", "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "data": { "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001" } }
```

```json
{ "v": 1, "id": "0192a6f4-9200-7c3a-9d2e-000000000031", "type": "task.started", "ts": "2026-09-19T15:04:05.470Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002", "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "data": { "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001", "startedAt": "2026-09-19T15:04:05.470Z" } }
```

### `task.completed` con `ToolResult`

```json
{
  "v": 1,
  "id": "0192a6f4-9300-7c3a-9d2e-000000000032",
  "type": "task.completed",
  "ts": "2026-09-19T15:04:06.900Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "data": {
    "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001",
    "completedAt": "2026-09-19T15:04:06.880Z",
    "result": {
      "schema": "jarvis.tool-result/1",
      "tool": "open_app",
      "toolVersion": "1.0.0",
      "executionId": "0192a6f4-9250-7d11-8000-00000000e001",
      "outcome": "SUCCEEDED",
      "startedAt": "2026-09-19T15:04:05.470Z",
      "finishedAt": "2026-09-19T15:04:06.880Z",
      "durationMs": 1410,
      "execution": { "status": "SUCCEEDED", "durationMs": 382 },
      "verification": {
        "status": "PASSED",
        "method": "process_and_window",
        "durationMs": 1028,
        "checks": [
          { "name": "process_running", "passed": true },
          { "name": "top_level_window_visible", "passed": true }
        ]
      },
      "output": { "appId": "chrome", "processId": 4312, "alreadyRunning": false },
      "truncated": false
    }
  }
}
```

### `task.failed` por verificación

```json
{
  "v": 1,
  "id": "0192a6f4-9300-7c3a-9d2e-000000000033",
  "type": "task.failed",
  "ts": "2026-09-19T15:04:16.000Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "data": {
    "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001",
    "error": {
      "code": "VERIFICATION_FAILED",
      "message": "Process started but exited before a window appeared (exit code 1).",
      "retryable": true
    }
  }
}
```

El `ToolResult` completo (con `verification.checks`) de una ejecución fallida se conserva localmente y se incluye en diagnósticos; al Cloud solo va el objeto `error` porque `task.failed` no tiene campo `result` en v1.

### `task.rejected` por permisos

```json
{ "v": 1, "id": "…", "type": "task.rejected", "ts": "…", "deviceId": "…", "correlationId": "…",
  "data": { "taskId": "…", "error": { "code": "NOT_PERMITTED", "message": "principal cloud:device:0192…0001 lacks windows.apps.close", "retryable": false } } }
```

El mensaje identifica el permiso faltante pero **no** revela la política completa.

### `task.approval_required`

```json
{ "v": 1, "id": "…", "type": "task.approval_required", "ts": "…", "deviceId": "…", "correlationId": "…",
  "data": { "taskId": "…", "reason": "close_app with force=true is R3 and requires local approval" } }
```

### `task.progress` (cancelación no soportada o avance)

```json
{ "v": 1, "id": "…", "type": "task.progress", "ts": "…", "deviceId": "…", "correlationId": "…",
  "data": { "taskId": "…", "percent": null, "message": "cancellation not supported by tool" } }
```

### `task.cancelled`

```json
{ "v": 1, "id": "…", "type": "task.cancelled", "ts": "…", "deviceId": "…", "correlationId": "…",
  "data": { "taskId": "…", "reason": "USER_REQUEST" } }
```

Razones usadas: `USER_REQUEST` (por `task.cancel`), `APPROVAL_DENIED`, `APPROVAL_EXPIRED` ([C9](../00-overview/cloud-compatibility.md)), `LOCAL_POLICY` (cancelada por cambio de política local antes de ejecutar).

## Límites

| Límite | Valor | Aplicado en |
|---|---|---|
| Envelope entrante | 64 KiB | Cloud Link |
| `payload` | 16 KiB (lo impone el Cloud; el agente lo revalida) | Pipeline etapa 7 |
| `result` (`ToolResult` serializado) | ≤ 32 KiB; `output` ≤ 24 KiB por contrato de tool | Tool Contract |
| `message` de error | ≤ 500 caracteres, sin datos sensibles | Tool Contract |
