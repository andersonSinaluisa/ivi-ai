# MVP — JARVIS WINDOWS AGENT MVP COMPLETE

El MVP está completo **solo si todas** las condiciones se cumplen y están verificadas.

## Tools incluidas

| Tool | TaskType (Cloud) | Riesgo | Por qué entra al MVP |
|---|---|---|---|
| `is_app_running` | `IS_APP_RUNNING` | R0 | Base de la verificación de `open_app`/`close_app`; sin datos sensibles |
| `list_running_apps` | `LIST_RUNNING_APPS` | R0 | Observación general; solo apps con ventana, sin rutas ni argumentos |
| `get_active_window` | `GET_ACTIVE_WINDOW` | R1 | Contexto para el futuro Agent Core; título redactado por defecto |
| `open_app` | `OPEN_APP` (alias `OPEN_CHROME`) | R2 | Caso principal del vertical slice |
| `close_app` | `CLOSE_APP` | R2 (graceful) / R3 (`force`) | Contraparte verificable de `open_app` |

Fuera del MVP (siguiente fase, diseño ya definido): `list_directory`, `read_file`, `run_command` (comandos con nombre), `focus_window`. Motivo: añaden superficies de exfiltración (filesystem, salida de comandos) cuyo sandbox merece su propia fase con pruebas de seguridad específicas; el vertical slice no las necesita.

## Demostración obligatoria

```text
Jarvis Cloud ──MQTT──► Windows Agent ──► open_app({"appId":"chrome"}) ──► Windows
                                            │
                                  verify: proceso chrome.exe vivo
                                  y ventana top-level visible
                                            │
Jarvis Cloud ◄──MQTT── task.completed (verification.status = PASSED)
```

1. Agente instalado por usuario, arranca al iniciar sesión, provisionado con su credencial en Credential Manager.
2. El Cloud muestra el device `Online` con `supportedTaskTypes` = tipos habilitados localmente.
3. `POST /api/v1/tasks` con `OPEN_APP {"appId":"chrome"}` → Chrome abierto → tarea `Completed` con `result.verification.status = "PASSED"` y `executionId`.
4. Idem para `IS_APP_RUNNING`, `LIST_RUNNING_APPS`, `GET_ACTIVE_WINDOW`, `CLOSE_APP`.
5. El botón del ESP32 Simulator (`OPEN_CHROME`) abre Chrome a través del alias.

## Criterios obligatorios

| Área | Criterio |
|---|---|
| Seguridad | Task de un principal sin la capability → `task.rejected NOT_PERMITTED`; R4 remoto siempre denegado; ningún camino ejecuta texto remoto como comando |
| Idempotencia | 100 redeliveries del mismo `task.dispatch` → 1 ejecución (criterio heredado de Cloud Phase 2) |
| Verificación | `open_app` de una app que se cierra al instante → `Failed VERIFICATION_FAILED`, no `Completed` |
| Recuperación | Resultado generado sin conexión se publica tras reiniciar la PC; tarea interrumpida por crash → `Failed EXECUTION_INTERRUPTED`, nunca re-ejecutada |
| Cancelación | `task.cancel` antes de iniciar → `task.cancelled`; durante `close_app` (espera) → cancelada |
| Timeouts | Ninguna Tool excede su `maxTimeout`; `TOOL_TIMEOUT` reportado |
| Presencia | LWT al matar el proceso; `Locked` reflejado en el estado local y en `sessionState` |
| Credenciales | Ningún secreto en archivos de configuración ni logs; revocación → backoff y estado `AUTH_REJECTED` visible |
| Observabilidad | Logs JSON rotados con `taskId`/`executionId`/`correlationId`; `jarvis-agent status` muestra health |
| Pruebas | Suites unit, integración (broker real) y Windows integration verdes; escenarios de [Integration Scenarios](../11-testing/integration-scenarios.md) WIN-E2E-01 … 14 |
| Empaquetado | Binario firmado; `jarvis-agent install` registra la Scheduled Task; `uninstall` limpia todo salvo datos si se pide |

## Explícitamente fuera del MVP

UI de aprobaciones, aprobación remota, firma end-to-end de tareas, UI Automation, navegador, visión, filesystem, comandos, actualizaciones automáticas, IA.
