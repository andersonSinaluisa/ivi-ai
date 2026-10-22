# EPIC-06 — Presence

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [MQTT Protocol §6 Presence](../../03-protocols/mqtt-protocol.md#6-presence).

---

## HU-035 — `presence.hello` y reconciliación de tareas abiertas

- **Objetivo:** registrar la conexión de un device y reconciliar sus tareas.
- **Descripción:** handler de `presence.hello`: actualiza `current_session_id`, `presence_state = Online`, `last_connected_at`, `last_seen_at`, `capabilities`, `supported_task_types`, `reported_info`; evento `DEVICE_CONNECTED`; responde `presence.hello.ack`; aplica la tabla de reconciliación con `openTaskIds` (`Dispatched` ausente → `Pending`; `Delivered`/`Running`/`WaitingForApproval` ausente → `Failed DEVICE_LOST`); despierta al dispatcher.
- **Criterios de aceptación:**
  1. Tras hello, `GET /api/v1/devices/{id}` muestra `Online` y las capabilities reportadas.
  2. `presence.hello.ack` llega con `heartbeatIntervalSeconds = 30` y el mismo `sessionId`.
  3. Cada fila de la tabla de reconciliación tiene su test.
  4. `openTaskIds` con más de 256 elementos → se procesan los primeros 256 y log `Warning`.
  5. Ids de `openTaskIds` que no pertenecen al device se ignoran (log `Warning`, sin revelar nada).
- **Dependencias:** HU-022, HU-023, HU-024.
- **Consideraciones técnicas:** la reconciliación y la actualización de presencia en una transacción.
- **Pruebas esperadas:** integración con MQTT real.
- **Definition of Done:** DoD común.

---

## HU-036 — Heartbeat, goodbye y LWT

- **Objetivo:** mantener `last_seen_at`/`busy` y detectar desconexiones.
- **Descripción:** `presence.heartbeat` actualiza `last_seen_at` y `reported_busy`; cualquier mensaje `up` con `sessionId` actual actualiza `last_seen_at` y recupera `Stale`/`Offline` → `Online`; `presence.goodbye` y LWT `presence.offline` con `sessionId` actual → `Offline` + `DEVICE_DISCONNECTED`; con `sessionId` distinto → ignorados.
- **Criterios de aceptación:**
  1. Matar el proceso de un device (sin DISCONNECT) → `Offline` con `reason = lwt` en ≤ 90 s + margen.
  2. Reconexión rápida con nueva sesión: el LWT de la sesión vieja **no** marca Offline la nueva.
  3. `busy = true` en heartbeat → `status = Busy`.
  4. Goodbye → `Offline` con `reason = goodbye` inmediato.
- **Dependencias:** HU-035.
- **Consideraciones técnicas:** escritura por heartbeat aceptable (ver [ADR-008](../../08-decisions/ADR-008-redis-decision.md)).
- **Pruebas esperadas:** integración con MQTTnet simulando corte abrupto (cerrar socket) y reconexión.
- **Definition of Done:** DoD común.

---

## HU-037 — Presence sweeper

- **Objetivo:** detectar devices silenciosos aunque no llegue LWT.
- **Descripción:** `PresenceSweeper` cada 15 s: `Online` sin mensajes > 75 s → `Stale` (notificación `device.stale`, sin evento); `Stale` sin mensajes > 180 s → `Offline` + `DEVICE_DISCONNECTED (heartbeat_timeout)`.
- **Criterios de aceptación:**
  1. Transiciones con los umbrales configurados (tests con reloj falso).
  2. Un mensaje del device en `Stale` lo devuelve a `Online` sin evento `DEVICE_CONNECTED`; desde `Offline`, con `DEVICE_CONNECTED (recovered)`.
  3. Heartbeat del servicio visible en readiness.
- **Dependencias:** HU-036.
- **Consideraciones técnicas:** índice `ix_devices_presence_sweep`.
- **Pruebas esperadas:** integración con `TimeProvider` falso.
- **Definition of Done:** DoD común.
