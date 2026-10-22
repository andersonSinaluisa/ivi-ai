# Presence

El Cloud calcula la presencia (Online/Stale/Offline) a partir de `hello`, heartbeats y LWT ([Cloud MQTT §6](../../../api/docs/03-protocols/mqtt-protocol.md#6-presence)). El agente aporta además el **estado de la sesión de Windows**, que es una dimensión distinta.

## Dos dimensiones separadas

| Dimensión | Valores | Quién la determina | Por qué separarla |
|---|---|---|---|
| **Conectividad del device** | `ONLINE`, `OFFLINE` (+ `STALE` calculado por el Cloud), `DISABLED` (admin en el Cloud) | Cloud | Refleja si el agente puede recibir tareas |
| **Actividad** | `busy: true/false` | Agente (ejecuciones en curso) | Ya existe en el protocolo |
| **Sesión de Windows** | `Active`, `Locked`, `Idle`, `Disconnected` | Agente (WTS + `GetLastInputInfo`) | Un device `ONLINE` con sesión `Locked` recibe tareas, pero ciertas tools no pueden ejecutarse ([Session Model](../05-windows/session-model.md)) |

Mezclar `LOCKED` o `IDLE` con `ONLINE/OFFLINE` haría que el Cloud creyera que el device no puede recibir nada, cuando en realidad las tools de observación y lanzamiento siguen funcionando.

## `presence.hello`

```json
{
  "v": 1,
  "id": "0192a6f4-9700-7c3a-9d2e-000000000070",
  "type": "presence.hello",
  "ts": "2026-09-19T15:00:00.000Z",
  "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "data": {
    "sessionId": "s-7f3a9c21e4b0a912",
    "protocolVersion": "1.1",
    "softwareVersion": "jarvis-windows-agent/0.1.0",
    "platform": "windows",
    "capabilities": ["agent", "windows-control"],
    "supportedTaskTypes": ["IS_APP_RUNNING", "LIST_RUNNING_APPS", "GET_ACTIVE_WINDOW", "OPEN_APP", "CLOSE_APP", "OPEN_CHROME"],
    "openTaskIds": ["0192a6f4-9000-7c3a-9d2e-00000000a001"],
    "busy": false,
    "sessionState": "Active"
  }
}
```

| Campo | Regla |
|---|---|
| `softwareVersion` | `jarvis-windows-agent/<semver>`; sin commit ni build path |
| `platform` | `"windows"` (sin edición, build ni arquitectura: no aportan al Cloud y facilitan fingerprinting) |
| `capabilities` | Descriptivas del catálogo del Cloud: `agent`, `windows-control`; `notifications` solo cuando exista `SHOW_NOTIFICATION` |
| `supportedTaskTypes` | Tools **habilitadas** en `config.toml` + alias. No depende de la política por principal |
| `openTaskIds` | No terminales + terminales sin `task.update.ack` (máx. 256, los más recientes) |
| `sessionState` | Extensión aditiva ([C11](../00-overview/cloud-compatibility.md)); el Cloud actual la ignora |

## `presence.heartbeat`

```json
{ "v": 1, "id": "…", "type": "presence.heartbeat", "ts": "…", "deviceId": "…",
  "data": { "sessionId": "s-7f3a9c21e4b0a912", "busy": true, "openTaskCount": 1, "sessionState": "Locked" } }
```

Cada `heartbeatIntervalSeconds` (del `hello.ack`, por defecto 30 s). Además se envía un heartbeat inmediato cuando cambia `busy` o `sessionState` (máx. 1 cada 5 s).

## Datos que **no** se publican (privacidad)

| Dato | Motivo |
|---|---|
| Hostname, dominio, usuario de Windows | Identifican a la persona y la red; el Cloud ya tiene `deviceId` y nombre ([C13](../00-overview/cloud-compatibility.md)) |
| IP local, MAC, lista de interfaces | Fingerprinting de red |
| Lista de apps instaladas o en ejecución | Solo bajo demanda con `list_running_apps` (autorizado) |
| Título de la ventana activa | Solo bajo demanda con `get_active_window` (autorizado y redactado) |
| Versión exacta de Windows / parches | Facilita elegir exploits |

## `goodbye` y LWT

- Apagado ordenado (`WM_QUERYENDSESSION`, `stop`, cierre de sesión): `presence.goodbye { reason: "shutdown" | "logoff" | "update" }` y `DISCONNECT` normal.
- Caída: el broker publica el LWT `presence.offline { sessionId }`.
- Suspensión/hibernación: se trata como caída (el broker detecta el keepalive vencido); al reanudar, reconexión con nueva sesión y hello.
