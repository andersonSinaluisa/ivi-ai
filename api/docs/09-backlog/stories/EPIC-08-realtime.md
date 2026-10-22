# EPIC-08 — Realtime

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [Realtime Protocol](../../03-protocols/realtime-protocol.md), [ADR-004](../../08-decisions/ADR-004-rest-signalr-responsibilities.md).

---

## HU-041 — Hub SignalR con autenticación y groups

- **Objetivo:** `/hubs/events` para clientes UI.
- **Descripción:** hub sin métodos de servidor que modifiquen estado; auth JWT de User (header o `access_token` en query **solo** para esta ruta); al conectar, agregar a `owner:{ownerId}`; cerrar conexiones cuando el token supera `exp + 1 h`.
- **Criterios de aceptación:**
  1. Sin token → 401; token de Device → 403.
  2. Un cliente no puede unirse a groups de otro owner (no existe método para hacerlo).
  3. El query string no aparece en logs de la API.
  4. Transportes: WebSockets y Long Polling; SSE deshabilitado.
- **Dependencias:** HU-009.
- **Consideraciones técnicas:** `JwtBearerEvents.OnMessageReceived` filtrando por path.
- **Pruebas esperadas:** integración con el cliente .NET de SignalR.
- **Definition of Done:** DoD común.

---

## HU-042 — Publicación de notificaciones realtime

- **Objetivo:** convertir eventos confirmados en notificaciones SignalR según el mapeo de [Event Model](../../02-domain/event-model.md#mapeo-a-notificaciones-realtime).
- **Descripción:** `SignalRRealtimeNotifier` suscrito al dispatcher post-commit de HU-038; envía `event` con `seq`, `name`, `occurredAt`, `correlationId`, `deviceId`, `taskId`, `data` reducida (sin `payload`/`result`); `device.stale` sin `seq` desde el sweeper.
- **Criterios de aceptación:**
  1. Crear y completar una tarea genera las notificaciones `task.*` en orden al cliente del owner, y ninguna al cliente de otro owner.
  2. Un cliente que se desconecta y hace catch-up con `after` obtiene exactamente los eventos perdidos (test del flujo documentado).
  3. Un fallo al enviar notificaciones no afecta al caso de uso.
- **Dependencias:** HU-041, HU-038.
- **Consideraciones técnicas:** envío asíncrono fuera de la transacción.
- **Pruebas esperadas:** integración con cliente SignalR .NET.
- **Definition of Done:** DoD común.
