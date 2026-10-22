# Process Tools (MVP)

Las cinco tools del MVP. Implementación Win32 en [Process Management](../05-windows/process-management.md) y [Active Window](../05-windows/active-window.md).

## `is_app_running` — R0

| Aspecto | Valor |
|---|---|
| Permiso | `windows.apps.read` |
| Input | `{ "appId": string }` — id del catálogo, nombre visible o alias (`chrome`, `Chrome`, `Google Chrome`); nunca una ruta |
| Output | `{ "appId", "running": bool, "processCount": int, "processes": [{ "pid", "name" }] }` (máx. 50, solo de la sesión del usuario) |
| Timeouts | default 2 s, max 5 s |
| Cancelación | `none` (demasiado corta) |
| Verificación | `NOT_APPLICABLE`: la observación **es** el resultado |
| Sesión | No requiere sesión interactiva |

Solo cuenta procesos de la **sesión del usuario** del agente (no de otras sesiones RDP/usuarios).

## `list_running_apps` — R0

| Aspecto | Valor |
|---|---|
| Permiso | `windows.apps.read` |
| Input | `{ "includeUncatalogued": bool = false, "limit": int 1..200 = 100 }` |
| Output | `{ "apps": [ { "appId"?, "processName", "processCount", "hasVisibleWindow" } ], "truncated" }` |
| Timeouts | default 3 s, max 10 s |
| Verificación | `NOT_APPLICABLE` |

- Por defecto devuelve solo apps del catálogo. `includeUncatalogued=true` añade nombres de ejecutable de procesos **con ventana visible** en la sesión (no servicios, no procesos de sistema).
- **Nunca** devuelve líneas de comandos, rutas completas, PIDs de otros usuarios ni variables de entorno.

## `get_active_window` — R1

| Aspecto | Valor |
|---|---|
| Permiso | `windows.window.read`; el título requiere además `windows.window.title` |
| Input | `{ "includeTitle": bool = false }` |
| Output | `{ "appId"?, "process": "Code.exe", "pid": 1234, "title"?: "Jarvis - Visual Studio Code", "titleRedacted": bool, "sessionState" }` |
| Timeouts | default 1 s, max 3 s |
| Sensibilidad | `output` (el título se redacta en BD tras *settled*) |
| Verificación | `NOT_APPLICABLE` |

Con sesión `Locked` devuelve `{ "sessionState": "Locked" }` sin ventana (no es error). Reglas de privacidad del título en [Active Window](../05-windows/active-window.md).

## `open_app` — R2

| Aspecto | Valor |
|---|---|
| Permiso | `windows.apps.open` |
| Input | `{ "appId": string, "ifRunning": "noop" \| "new_instance" = "noop" }` (`focus` llegará con `focus_window` en P7: no se acepta lo que todavía no se puede verificar) |
| Output | `{ "appId", "processId"?, "alreadyRunning": bool, "action": "launched" \| "none" \| "focused" }` |
| Recursos | `APP:<appId>`; `DESKTOP_INPUT` solo si `ifRunning = "focus"` |
| Timeouts | default 15 s, max 60 s |
| Cancelación | `cooperative` antes del lanzamiento; una vez lanzada la app no se revierte y la verificación se completa para reportar el estado real |
| Sesión | Requiere `Active` (lanzar una app con la sesión bloqueada no es observable para el usuario → `SESSION_UNAVAILABLE`) |
| Idempotencia de efecto | `ifRunning=noop`: si ya está corriendo, éxito sin lanzar |

No acepta argumentos de línea de comandos ni URLs en el MVP (evita convertir `open_app` en ejecución arbitraria: `chrome --load-extension=…`). Argumentos predefinidos por entrada del catálogo en el futuro.

**Verificación** (`process_and_window`):
1. Existe un proceso de la app en la sesión (el PID lanzado o, para apps de instancia única/broker, cualquier proceso de `process_names` creado después del lanzamiento).
2. Existe una ventana top-level visible, no *cloaked*, de ese conjunto de procesos.
3. Sondeo cada 250 ms hasta 10 s (configurable por app: algunas tardan).
4. Si el proceso lanzado sale con código ≠ 0 antes de que aparezca ventana → `VERIFICATION_FAILED` con el código de salida.

## `close_app` — R2 (R3 con `force`)

| Aspecto | Valor |
|---|---|
| Permiso | `windows.apps.close`; `force=true` requiere `windows.apps.close.force` |
| Input | `{ "appId": string, "force": bool = false, "gracePeriodMs": int 1000..30000 = 5000 }` |
| Output | `{ "appId", "closedProcesses": int, "forced": bool, "wasRunning": bool, "status": "NOT_RUNNING" \| "CLOSE_REQUESTED" \| "CLOSED" }` |
| Recursos | `APP:<appId>` |
| Timeouts | default 15 s, max 45 s |
| Cancelación | `cooperative` (entre fases) |

Estrategia:
1. `WM_CLOSE` a las ventanas top-level de los procesos de la app (permite al usuario guardar).
2. Esperar `gracePeriodMs`.
3. Si quedan procesos y `force=false` → `VERIFICATION_FAILED` ("app still running, may be waiting for user input"). **No** se escala solo.
4. Si `force=true` → `TerminateProcess` sobre los restantes (riesgo de pérdida de datos: por eso R3 y aprobación).
5. Procesos elevados (integridad alta) no se pueden cerrar desde integridad media → `APP_ELEVATED`.
6. App no en ejecución → éxito con `wasRunning=false` (idempotencia de efecto).

**Verificación** (`process_absent`): ningún proceso de `process_names` en la sesión, sondeando cada 250 ms hasta 5 s tras la acción.

## Nunca matar

Lista de denegación fija (no configurable desde el Cloud): `csrss.exe`, `wininit.exe`, `winlogon.exe`, `lsass.exe`, `services.exe`, `smss.exe`, `svchost.exe`, `dwm.exe`, `explorer.exe` (salvo entrada explícita del catálogo con `allow_close = true`), `MsMpEng.exe`, y el propio `jarvis-agent.exe`. Además, solo procesos cuyo SID de token coincida con el del usuario del agente.
