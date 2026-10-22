# Future Mobile / Web Contract

Aplica a la app mobile (React Native, Phase 9), a la UI web y a la UI Tauri (Phase 3).

## Dos identidades posibles

| Identidad | Para qué | Canal |
|---|---|---|
| **User** (login del owner) | Ver devices, tareas, eventos; administrar; aprobar (Phase 3) | REST + SignalR |
| **Device** (`Mobile`/`Web` registrado) | Actuar como origen de tareas con grants propios (p. ej. un widget que abre algo en la PC), recibir notificaciones MQTT | MQTT (opcional) o REST con device token |

Recomendación: la UI usa identidad **User**. Registrar además la app como Device solo si necesita comportamiento de device (origen con grants limitados, operación sin sesión de usuario, recibir tareas). Mantenerlas separadas permite revocar el teléfono como device sin cerrar la cuenta del owner.

## Flujo de UI

1. Login → access + refresh token (guardados en Keychain/Keystore; en web, refresh token en cookie `HttpOnly; Secure; SameSite=Strict` — **decisión pendiente** al construir la UI web, requiere agregar variante de `/auth/refresh` basada en cookie).
2. Carga inicial por REST (`/devices`, `/tasks`, `/events?limit=1` para `lastSeq`).
3. Conexión SignalR a `/hubs/events` con catch-up ([Realtime Protocol](../03-protocols/realtime-protocol.md)).
4. Acciones por REST (`POST /tasks`, `/cancel`, administración de devices).

## Notificaciones push (futuro)

Cuando la app está en background SignalR no funciona. Plan: un componente de notificaciones push (FCM/APNs) que se suscribe a eventos seleccionados (`approval.requested`, `task.failed`). Se agregará como un nuevo consumidor de `events` dentro de la API; no requiere cambios en el protocolo de devices.

## Voz (futuro)

La captura de voz en mobile produce texto (STT en el cliente o en un servicio propio) que se envía como tarea `ASSISTANT_REQUEST` al Windows device. El Cloud transporta el texto como cualquier `payload`; no procesa audio en MVP ni en Phase 9 salvo ADR nuevo.

## Requisitos para el Cloud (ya cubiertos por el diseño)

- CORS configurable por lista de orígenes (web).
- `?access_token=` en `/hubs/events` para WebSocket.
- Paginación por cursor y filtros en `/tasks` y `/events`.
- Errores Problem Details con `code` estable para mostrar mensajes localizados en el cliente.
