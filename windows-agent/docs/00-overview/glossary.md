# Glosario

Términos normativos del Windows Agent. Los términos de transporte (Task, TaskType, envelope, settled, openTaskIds…) se heredan del [glosario de Jarvis Cloud](../../../api/docs/00-overview/glossary.md) y no se redefinen.

| Término | Definición |
|---|---|
| **Agent** | El proceso `jarvis-windows-agent` en ejecución. |
| **Tool** | Unidad de capacidad con contrato declarado (nombre, riesgo, permisos, recursos, timeouts, esquemas, verificación). Único medio para producir efectos en Windows. Ver [Tool Contract](../03-tools/tool-contract.md). |
| **Tool name** | Identificador `snake_case` (`open_app`). Se corresponde 1:1 con un **TaskType** del Cloud en `UPPER_SNAKE_CASE` (`OPEN_APP`). |
| **Tool input / output** | JSON validado por esquema. El input viaja en `task.dispatch.data.payload`; el output en `task.completed.data.result.output`. |
| **Task** | Solicitud remota recibida por MQTT (`task.dispatch`), identificada por `taskId` (asignado por el Cloud). |
| **Execution** | Único intento local de ejecutar la Tool de una Task. `executionId` (UUIDv7) generado por el agente. Una Task tiene **como máximo una** Execution. |
| **Invocation** | Llamada a una Tool desde cualquier origen: Cloud Task (hoy) o Agent Core local (futuro). |
| **Principal** | Quién solicita una invocación. Remotos: derivados de `requestedBy` del Cloud, con notación `cloud:user:<id>` / `cloud:device:<id>` (comodín `cloud:*`); hasta la firma end-to-end son identidades **afirmadas** por el Cloud. Locales: `local:owner` (CLI) y `local:agent-core` (futuro). Ver [Permission Engine](../04-security/permission-engine.md#principals). |
| **Capability (permiso local)** | Permiso nombrado (`windows.apps.open`) que la política local concede a un principal. No confundir con las *capabilities* descriptivas que el device anuncia en `presence.hello`. |
| **Policy** | Archivo local `policy.toml` con principals, capabilities, umbrales de riesgo y reglas de aprobación. Autoridad final. |
| **Risk level** | `R0`…`R4`, declarado por cada Tool (y ajustable por input, p. ej. `close_app` con `force`). Ver [Risk Model](../04-security/risk-model.md). |
| **Decision** | Resultado del Permission Engine: `APPROVED`, `DENIED`, `REQUIRES_APPROVAL`. |
| **Approval** | Registro local que autoriza una Task en `REQUIRES_APPROVAL`. Estados: `Pending`, `Granted`, `Denied`, `Expired`. |
| **Execution status** | Resultado de ejecutar: `SUCCEEDED`, `FAILED`, `TIMED_OUT`, `CANCELLED`. |
| **Verification status** | Resultado de observar: `PASSED`, `FAILED`, `INCONCLUSIVE`, `NOT_APPLICABLE` (y `SKIPPED` cuando la ejecución falló y no se verificó). |
| **Observation** | Lectura del estado real de Windows tras ejecutar (proceso existe, ventana visible, proceso terminó…). |
| **Resource lock** | Recurso lógico que una Tool declara (`DESKTOP_INPUT`, `APP:<appId>`, `FS:<ruta>`, `COMMAND:<id>`) para serializar ejecuciones conflictivas. |
| **App catalog** | Lista local de aplicaciones permitidas (`appId` → ejecutable Win32 o AUMID de app empaquetada). El input remoto nunca es una ruta. |
| **Named command** | Comando definido localmente (`git.status`) con programa, argumentos fijos y parámetros validados. Único modo de ejecutar procesos arbitrarios. |
| **Allowed root** | Directorio permitido para tools de filesystem, canonicalizado al cargar la política. |
| **Outbox** | Mensajes de estado pendientes de confirmación (persistidos) que se reenvían tras reconectar hasta `task.update.ack`. |
| **Session state** | Estado de la sesión de Windows: `Active`, `Locked`, `Idle`, `Disconnected`. Independiente de la conectividad del device. |
| **Tool Host** | Componente que expone el registro de tools a los *frontends* (Cloud Task Adapter y, en el futuro, Local Tool API). |
| **Tool Provider** | Proveedor de tools; `builtin` (en proceso) o externo (proceso separado, p. ej. navegador con Playwright). |
