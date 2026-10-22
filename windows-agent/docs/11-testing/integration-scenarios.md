# Integration Scenarios (E2E)

Escenarios de extremo a extremo **contra Jarvis Cloud real** (repo `api/`, `docker compose`) y el agente en el runner Windows interactivo. Todos forman parte de la aceptación del [MVP](../00-overview/mvp.md).

Preparación común: Cloud en marcha; device Windows registrado y provisionado (`jarvis-agent provision`); `policy.toml` de test con: owner (`cloud:user:<owner>`) con capabilities de procesos incluida `close.force`; device ESP32 simulado con `windows.apps.read` + `windows.apps.open`; `cloud:*` sin capabilities. Catálogo con `chrome`, `notepad` y `testapp` (`jarvis-test-app.exe`).

Comprobaciones: estado de la tarea en `GET /api/v1/tasks/{id}`, BD local (`tasks`, `executions`, `outbox`) y logs.

## Cómo se ejecutan (implementado)

[`scripts/e2e.ps1`](../../scripts/e2e.ps1) registra un device nuevo en el Cloud, levanta un agente con raíz temporal propia, recorre los catorce escenarios y archiva la evidencia (respuestas de la API, `config.toml`, `policy.toml`, logs del agente y estado final) en `scripts/e2e-evidence/<timestamp>/`.

```powershell
pwsh -File scripts/e2e.ps1 -Email owner@example.com -Password '<passphrase>'
```

Reglas del runner:

- un escenario que no se puede ejecutar en el entorno (falta una app, el broker no es controlable, el Cloud no acepta ese tipo) se marca **SKIP con el motivo**, nunca PASS;
- el device de pruebas se revoca al terminar, salvo `-KeepDevice`;
- WIN-E2E-03 necesita la credencial MQTT del Cloud (`api/.local-secrets/mqtt_cloud_password`) para publicar duplicados como lo haría el Cloud.

Última ejecución (2026-09-19): **14 PASS, 0 FAIL, 0 SKIP**; evidencia en `scripts/e2e-evidence/20260919-213733/`. En WIN-E2E-03, 100 publicaciones del mismo `task.dispatch` produjeron **una** ejecución y 49 respuestas con el último estado (el resto lo descartó el limitador de entrada). Los fallos que destapó esta suite están en [Estado de implementación](../10-backlog/implementation-status.md#hallazgos-de-la-ejecución-contra-el-cloud-real).

### WIN-E2E-01 Tarea válida `open_app`

| | |
|---|---|
| Pasos | Owner: `POST /api/v1/tasks` `OPEN_APP {"appId":"notepad"}` (y `chrome` si está instalado) |
| Esperado | Ventana abierta. Cloud: `Pending → Dispatched → Delivered → Running → Completed`; `result.schema = jarvis.tool-result/1`, `outcome = SUCCEEDED`, `verification.status = PASSED`, `executionId` presente |
| Cubre | valid task, execution success, verificación |

### WIN-E2E-02 Tarea inválida

| | |
|---|---|
| Pasos | `OPEN_APP {"appId":"../../cmd"}`; `OPEN_APP {"app":"chrome"}` (campo desconocido); `OPEN_APP {"appId":"no-existe"}` |
| Esperado | `Rejected` con `INVALID_PAYLOAD` en los tres; `message` indica el motivo; nada se ejecuta |

### WIN-E2E-03 Tarea duplicada

| | |
|---|---|
| Pasos | Un "Cloud de test" publica 100 veces el mismo `task.dispatch` (`OPEN_APP testapp`), incluidas entregas tras reconexión |
| Esperado | 1 proceso `testapp` lanzado; 1 fila en `executions`; cada duplicado recibe el último estado persistido |

### WIN-E2E-04 Device revocado

| | |
|---|---|
| Pasos | Rotar/revocar la credencial del device en el Cloud |
| Esperado | Desconexión, `AUTH_REJECTED` en `status`, reintentos cada 5 min; tras `provision` con la nueva credencial → `Online` sin reiniciar el agente |

### WIN-E2E-05 Permiso denegado

| | |
|---|---|
| Pasos | Device ESP32 solicita `CLOSE_APP {"appId":"notepad"}` (no tiene `windows.apps.close`); solicitud R4 simulada con una tool de test R4 |
| Esperado | `Rejected NOT_PERMITTED`, `message` con la capability faltante; R4 → `NOT_PERMITTED` siempre; ningún efecto |

### WIN-E2E-06 Tool inexistente

| | |
|---|---|
| Pasos | Tarea `FORMAT_DISK {}` (el Cloud la despacha si el device la declarara; se usa el Cloud de test para forzar el dispatch) y `READ_FILE` con la tool deshabilitada |
| Esperado | `Rejected UNSUPPORTED_TASK_TYPE`; `supportedTaskTypes` del hello no incluye tools deshabilitadas |

### WIN-E2E-07 Ejecución fallida

| | |
|---|---|
| Pasos | Entrada de catálogo `broken` cuyo ejecutable fue borrado; `close_app` sobre `testapp` lanzada elevada |
| Esperado | `Failed APP_NOT_FOUND` y `Failed APP_ELEVATED` respectivamente; sin stack traces en el mensaje |

### WIN-E2E-08 Verificación fallida

| | |
|---|---|
| Pasos | `OPEN_APP testapp` configurada para salir con código 1 antes de mostrar ventana; `CLOSE_APP testapp` con confirmación de cierre activada y `force=false` |
| Esperado | Ambas `Failed VERIFICATION_FAILED` (con `exitCode` / "app still running"); **nunca** `Completed` |

### WIN-E2E-09 Desconexión durante la ejecución

| | |
|---|---|
| Pasos | `OPEN_APP testapp` con ventana retrasada 5 s; cortar la red del agente (proxy TCP) durante la ejecución; restaurar a los 30 s |
| Esperado | La tarea termina localmente; resultado en outbox; al reconectar, `openTaskIds` la incluye, se publica `task.completed`, llega `task.update.ack`, queda *settled*. El Cloud no la marca `DEVICE_LOST` |

### WIN-E2E-10 Reinicio del agente con trabajo pendiente

| | |
|---|---|
| Pasos | (a) Terminar la tarea sin conexión y reiniciar el proceso antes de reconectar. (b) Matar el proceso (`TerminateProcess`) con una tarea en `Running` |
| Esperado | (a) El resultado se publica tras el reinicio. (b) `Failed EXECUTION_INTERRUPTED`, sin re-ejecución, aunque el Cloud reenvíe el dispatch |

### WIN-E2E-11 Cancelación

| | |
|---|---|
| Pasos | (a) `POST /tasks/{id}/cancel` de una tarea en espera de aprobación. (b) Cancelar `close_app testapp` durante el período de gracia. (c) Cancelar una tool `none` (`is_app_running` con retraso de test) |
| Esperado | (a) `Cancelled USER_REQUEST` sin ejecutar. (b) `Cancelled`, `testapp` sigue abierta. (c) `task.progress` "cancellation not supported by tool" y estado terminal real |

### WIN-E2E-12 Timeout

| | |
|---|---|
| Pasos | `OPEN_APP testapp` con ventana retrasada 90 s (> `maxTimeout` 60 s); tarea con `maxDurationSeconds = 5` |
| Esperado | `Failed TOOL_TIMEOUT` en ≤ timeout efectivo + 1 s; nunca se excede `maxTimeout` |

### WIN-E2E-13 Aprobación local de una acción R3

| | |
|---|---|
| Pasos | Owner: `CLOSE_APP {"appId":"testapp","force":true}`. (a) `jarvis-agent approvals approve`. (b) `deny`. (c) Dejar expirar |
| Esperado | Cloud `WaitingForApproval`; (a) `Completed` con `forced: true`; (b) `Cancelled APPROVAL_DENIED`; (c) `Cancelled APPROVAL_EXPIRED` |

### WIN-E2E-14 Alias `OPEN_CHROME` desde el ESP32 Simulator

| | |
|---|---|
| Pasos | Botón del ESP32 Simulator del Cloud (`OPEN_CHROME`) |
| Esperado | Chrome abierto (o `notepad` si el catálogo de test redefine `chrome` en un runner sin Chrome); `Completed` con `result.tool = "open_app"` |

### WIN-E2E-15 Tarea firmada de extremo a extremo

| | |
|---|---|
| Pasos | `jarvis-sign` firma una tarea con la clave del owner; el Cloud la transporta; la política del agente exige firma por encima de R2 (`[signatures] require_above = "R2"`). Después, la misma tarea con un byte cambiado en `signature.value` |
| Esperado | La firmada llega a `Completed`. La alterada llega igualmente al agente —el Cloud no verifica firmas, no tiene la clave— y **el agente** la rechaza con `Failed SIGNATURE_INVALID` |

### WIN-E2E-16 Aprobación remota firmada

| | |
|---|---|
| Pasos | Tarea R3 firmada → el agente responde `task.approval_required` → el owner firma la decisión con `jarvis-sign decision --granted` → `POST /api/v1/tasks/{id}/approval` |
| Esperado | Cloud `WaitingForApproval`; la decisión se transmite (202); el agente verifica firma, nonce y frescura, la aplica y la tarea termina en `Completed`. En el event log quedan `APPROVAL_DECISION_SENT` (Cloud) y `APPROVAL_GRANTED` (device) como hechos distintos |
