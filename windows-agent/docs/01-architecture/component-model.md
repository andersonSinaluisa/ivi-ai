# Component Model

Responsabilidades, entradas/salidas y qué **no** hace cada componente. Nombres usados en todo el proyecto.

## Cloud Link

| | |
|---|---|
| Responsabilidad | Conexión MQTT/TLS, `CONNECT` con LWT, suscripción `down/#`, `presence.hello`/`hello.ack`, heartbeat, `goodbye`, reconexión con backoff, publicación de mensajes `up/*`, vaciado del outbox tras `Ready` |
| Entradas | `task.dispatch`, `task.cancel`, `task.update.ack`, `presence.hello.ack`, `request.*` (ignorados salvo log) |
| Salidas | Eventos internos `DispatchReceived`, `CancelReceived`, `UpdateAcked`, `LinkState` |
| No hace | No decide nada sobre tareas; no conoce tools |
| Detalle | [MQTT Client](../02-cloud-integration/mqtt-client.md), [Presence](../02-cloud-integration/presence.md), [Reconnection](../02-cloud-integration/reconnection.md) |

## Task Pipeline

| | |
|---|---|
| Responsabilidad | Orquestar cada tarea: deserializar, validar, dedup, expiración, resolver tool, validar input, pedir decisión a Policy, persistir, `task.ack`, programar ejecución, publicar estados, manejar `task.cancel` y aprobaciones |
| Estado | Máquina de estados local ([Execution Pipeline](execution-pipeline.md#estados-locales)) persistida en State Store |
| No hace | No ejecuta tools directamente (lo hace el Executor); no habla MQTT (lo hace Cloud Link) |

## Policy

| | |
|---|---|
| Responsabilidad | Cargar y validar `policy.toml`; evaluar `(principal, tool, input, riskEfectivo)` → `APPROVED \| DENIED \| REQUIRES_APPROVAL` con motivo; gestionar aprobaciones (crear, expirar, decidir por CLI) |
| No hace | No confía en capabilities enviadas por el Cloud; no tiene estado remoto |
| Detalle | [Permission Engine](../04-security/permission-engine.md), [Risk Model](../04-security/risk-model.md), [Approvals](../04-security/approvals.md) |

## Tool Host

| | |
|---|---|
| Responsabilidad | Registro de tools builtin (y futuros providers externos); mapeo TaskType ↔ tool; descriptores con JSON Schema; lista `supportedTaskTypes` para el hello |
| Frontends | Task Pipeline (Cloud) hoy; Local Tool API (Agent Core) en P10 — **misma instancia, misma política** |
| Detalle | [Tool System](../03-tools/tool-system.md), [Tool Registry](../03-tools/tool-registry.md) |

## Executor

| | |
|---|---|
| Responsabilidad | Ejecutar una invocación autorizada: adquirir resource locks, aplicar timeout efectivo, propagar cancelación, capturar resultado/errores, invocar al Verifier, medir duración |
| Garantías | Nunca más de una ejecución por `taskId`; nunca excede `maxTimeout` |
| Detalle | [Concurrency Model](concurrency-model.md) |

## Verifier

| | |
|---|---|
| Responsabilidad | Tras ejecutar, observar el estado real con la condición declarada por la tool (sondeo con intervalo y límite), producir `verification.status` |
| Principio | *Una acción se considera completada únicamente cuando el resultado observable satisface la condición de éxito definida por la Tool.* |
| Detalle | [Verification](../03-tools/verification.md) |

## Tools (builtin)

Implementan el [Tool Contract](../03-tools/tool-contract.md). Usan **solo** las interfaces de Platform; nunca `windows-rs` directamente (testeables con plataformas falsas).

## Platform (`jarvis-windows`)

| Interfaz | Operaciones |
|---|---|
| `ProcessPlatform` | Enumerar procesos, abrir/lanzar (CreateProcess sin shell, activación AUMID), cerrar (WM_CLOSE / TerminateProcess), consultar imagen, integridad y sesión de un PID |
| `WindowPlatform` | Ventana en primer plano, ventanas top-level visibles por PID, título, foco |
| `SessionPlatform` | Estado de sesión (WTS), idle (`GetLastInputInfo`), notificaciones lock/unlock |
| `AppCatalogPlatform` | App Paths del registro, paquetes instalados (AUMID) |
| `FileSystemPlatform` (P7) | Apertura segura con resolución de ruta final, atributos, reparse points |
| `CommandPlatform` (P7) | CreateProcess con Job Object, límites y captura de salida |
| `UiAutomationPlatform` (P9) | Hilo STA, árbol UIA, patrones |
| `CredentialPlatform` | `CredReadW`/`CredWriteW`/`CredDeleteW` |

## State Store

SQLite con un único escritor. Tablas y retención en [Local State](../06-state/local-state.md).

## Identity

Provisioning (`jarvis-agent provision`), lectura de la credencial para Cloud Link, manejo de `AUTH_REJECTED`. Ver [Credential Storage](../04-security/credential-storage.md).

## CLI local

`jarvis-agent <comando>` con los mismos binarios:

| Comando | Uso |
|---|---|
| `run` | Proceso residente (lo usa la Scheduled Task) |
| `provision --device-id <id> [--host h] [--port p]` | Guarda la credencial (secreto por stdin) y prueba la conexión |
| `status` | Health snapshot ([Health](../07-observability/health.md)) |
| `approvals list \| approve <taskId> \| deny <taskId>` | Decisión local de aprobaciones (sin UI) |
| `tools list` | Tools habilitadas, riesgo, permisos |
| `policy check [--principal p --tool t --input json]` | Valida `policy.toml`; con argumentos muestra la decisión y la regla aplicada a esa invocación |
| `policy reload` | Recarga la política en el proceso residente |
| `stop` | Apagado ordenado del proceso residente |
| `update` (P8) | Busca y aplica una actualización firmada ([Startup Model](../05-windows/startup-model.md#actualizaciones)) |
| `install` / `uninstall` | Registro de la Scheduled Task y directorios |
| `diagnostics` | Paquete de diagnóstico redactado |
| `version` | Versión, commit, protocolo |

El CLI se comunica con el proceso residente por un **named pipe local** con ACL restringida al usuario (`\\.\pipe\jarvis-agent-<SID>`), salvo `provision`, `install` y `policy check`, que operan sin el proceso.
