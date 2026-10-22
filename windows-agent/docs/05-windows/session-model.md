# Session Model

## Conceptos

| Concepto | Qué es | Relevancia |
|---|---|---|
| **Windows Session** | Sesión de Terminal Services (`SessionId`). Session 0 = servicios; 1+ = usuarios interactivos | El agente debe vivir en la sesión del usuario para ver sus ventanas |
| **Window Station / Desktop** | `WinSta0\Default` (escritorio del usuario), `WinSta0\Winlogon` (pantalla de bloqueo/login), Secure Desktop de UAC | Solo el escritorio `Default` es automatizable |
| **Locked Session** | Sesión existente con el escritorio `Winlogon` activo | Apps siguen corriendo; no hay UI accesible |
| **RDP** | Sesión remota; puede estar conectada o desconectada | Desconectada ≈ bloqueada (sin render ni input) |
| **Multiple users** | Fast User Switching: varias sesiones interactivas | Cada usuario tiene su propio agente y su propio device |
| **UAC Secure Desktop** | Escritorio aislado para el prompt de elevación | Inaccesible a propósito (ni UIA, ni input, ni captura) |

## Estado de sesión

El agente mantiene `sessionState`:

| Estado | Detección |
|---|---|
| `Active` | Sesión conectada, desbloqueada, con input reciente |
| `Idle` | `Active` pero sin input durante `idle_threshold` (default 10 min), vía `GetLastInputInfo` |
| `Locked` | `WTS_SESSION_LOCK` / `WTS_SESSION_UNLOCK` (`WTSRegisterSessionNotification`) |
| `Disconnected` | `WTS_REMOTE_DISCONNECT` / `WTS_CONSOLE_DISCONNECT` |

Se publica como campo aditivo en presencia ([Presence](../02-cloud-integration/presence.md)) y es independiente de `ONLINE/OFFLINE/BUSY/DISABLED`, que describen la conectividad del device con el Cloud.

## ¿Qué tools funcionan con la PC bloqueada?

| Tool | `Active`/`Idle` | `Locked` / `Disconnected` | Motivo |
|---|---|---|---|
| `is_app_running` | ✅ | ✅ | Solo enumera procesos |
| `list_running_apps` | ✅ | ✅ (`hasVisibleWindow` puede ser impreciso) | Enumeración |
| `get_active_window` | ✅ | ✅ devuelve `sessionState: Locked` sin ventana | No es un error |
| `open_app` | ✅ | ❌ `SESSION_UNAVAILABLE` | La app se lanzaría pero no puede verificarse ventana visible, y el usuario no la ve |
| `close_app` (graceful) | ✅ | ⚠️ `WM_CLOSE` funciona, pero un diálogo "¿Guardar?" no puede atenderse → `VERIFICATION_FAILED` | Se permite: la verificación decide |
| `close_app force` | ✅ | ✅ (R3, aprobación) | `TerminateProcess` no necesita UI |
| `list_directory` / `read_file` | ✅ | ✅ | Sin UI |
| `run_command` | ✅ | ✅ | Sin UI |
| `focus_window`, UIA, visión, input | ✅ | ❌ `SESSION_UNAVAILABLE` | Sin escritorio accesible |

Cada tool lo declara con `requiresInteractiveSession` ([Tool Contract](../03-tools/tool-contract.md)).

Política opcional `[session] deny_when_locked = ["open_app"]` o la inversa: el owner puede querer que **nada** con efectos ocurra con la PC bloqueada (nadie está mirando). Default: el comportamiento de la tabla.

## Idle

`Idle` no bloquea tools. Es informativo (el futuro Agent Core puede evitar interrumpir al usuario cuando está activo, o preferir ejecutar acciones visibles cuando está presente).

## Multiusuario y RDP

- Un agente **por usuario de Windows**, cada uno con su `deviceId` y su credencial (Credential Manager es por usuario).
- Instancia única **por sesión**: mutex `Local\Jarvis.Agent.<SID>`. Si el mismo usuario abre dos sesiones (consola + RDP), la segunda instancia no arranca y lo registra.
- Solo se observan y controlan procesos de la **propia** sesión y del propio SID.

## Suspensión e hibernación

`PBT_APMSUSPEND`: se deja de aceptar tareas nuevas, se envía `presence.goodbye` si hay tiempo. `PBT_APMRESUMEAUTOMATIC`: reconexión inmediata. Una tarea en `Running` durante la suspensión sigue su curso al reanudar; su timeout usa reloj monotónico, que en Windows **no** avanza durante la suspensión (`QueryUnbiasedInterruptTime`), y su `expiresAt` ya no importa porque la tarea había empezado.
