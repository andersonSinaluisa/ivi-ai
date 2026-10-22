# Health

## Health snapshot

El agente mantiene en memoria un snapshot actualizado por cada componente. Se consulta con `jarvis-agent status` (named pipe) y se resume en los heartbeats (`busy`, `sessionState`).

```json
{
  "agent": { "state": "RUNNING", "version": "0.1.0", "uptimeSeconds": 8123, "safeMode": false, "elevated": false },
  "mqtt": { "connected": true, "since": "2026-09-19T14:10:02Z", "reconnects24h": 1, "lastError": null },
  "cloud": { "reachable": true, "lastHelloAck": "2026-09-19T14:10:03Z", "clockOffsetMs": -212, "authRejected": false },
  "db": { "healthy": true, "sizeBytes": 1048576, "outboxPending": 0, "oldestUnsettledAgeSeconds": null, "lastCheck": "2026-09-19T14:10:00Z" },
  "tools": { "healthy": true, "enabled": 5, "unavailable": [], "running": 0, "queued": 0 },
  "policy": { "valid": true, "loadedAt": "2026-09-19T14:09:59Z", "hash": "sha256:…" },
  "session": { "state": "Active" },
  "conditions": [ ]
}
```

| Pedido | Campo | Cómo se determina |
|---|---|---|
| MQTT connected | `mqtt.connected` | Estado del Cloud Link (CONNACK recibido, sin desconexión posterior) |
| Cloud reachable | `cloud.reachable` | `presence.hello.ack` recibido en esta sesión y `task.update.ack`/heartbeat sin errores; distinto de MQTT: el broker puede aceptar la conexión con la API caída (sin hello.ack) |
| Agent running | `agent.state` | `STARTING` → `RUNNING` → `STOPPING`; `SAFE_MODE` |
| Local DB healthy | `db.healthy` | `quick_check` al arrancar, escrituras sin error, outbox sin atascos |
| Tool system healthy | `tools.healthy` | Registro válido, providers disponibles (futuro), hilo STA (P9) vivo, sin ejecuciones que superen su `maxTimeout` |

## Condiciones

Lista de condiciones activas con severidad. El estado global es el peor de ellas.

| Condición | Severidad | Origen | Resolución |
|---|---|---|---|
| `NOT_PROVISIONED` | ERROR | Sin credencial | `jarvis-agent provision` |
| `AUTH_REJECTED` | ERROR | CONNACK rechazado | Re-provisionar ([Reconnection](../02-cloud-integration/reconnection.md#autenticación-rechazada)) |
| `CLOUD_UNREACHABLE` | WARN | Sin conexión > 2 min | Automática al reconectar |
| `CLOCK_SKEW` | WARN | Desfase absoluto con el Cloud > 5 min | Sincronizar la hora de Windows |
| `POLICY_INVALID` | ERROR | `policy.toml` inválida (fail-closed) | Corregir y `policy reload` |
| `STATE_RESET` | WARN | BD recreada | Reconocer en `status` |
| `OUTBOX_STUCK` | WARN | Mensaje sin ack > 7 días | Revisar conectividad o el Cloud |
| `DB_ERROR` | ERROR | Errores de escritura (disco lleno, permisos) | Liberar espacio |
| `RUNNING_ELEVATED` | WARN | Token elevado | Ejecutar como usuario estándar |
| `SAFE_MODE` | ERROR | Crash loop | `status --clear-safe-mode` |
| `TOOL_STUCK` | ERROR | Ejecución que no terminó tras `maxTimeout` + gracia (no se pudo cancelar) | Diagnóstico; reinicio del agente |
| `UPDATE_ROLLED_BACK` | ERROR | La versión nueva no arrancó ni conectó y se volvió a la anterior | Revisar los logs de la versión nueva antes de reintentar |

## Qué ve el Cloud

El Cloud **no** recibe el snapshot completo en v1 (no hay mensaje para ello). Ve la presencia (hello/heartbeat/LWT), `busy` y `sessionState`. Un futuro `presence.health` resumido (solo condiciones, sin rutas ni tamaños) se propondría como cambio aditivo al protocolo.

## Salida de `jarvis-agent status`

```text
Jarvis Windows Agent 0.1.0            RUNNING (uptime 2h 15m)
Device     0192a6f0-…-000000000002
Cloud      connected (mqtts://jarvis.example.com:8883)  hello.ack 14:10:03  clock -0.2s
Session    Active
Tasks      running 0 · queued 0 · awaiting approval 1 · outbox 0
Policy     valid (loaded 14:09:59)
Conditions none
```

Código de salida de `status`: 0 si sano, 1 si hay WARN, 2 si hay ERROR (útil para scripts).
