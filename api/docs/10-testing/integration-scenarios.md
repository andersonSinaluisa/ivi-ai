# Integration Scenarios (E2E)

Escenarios automatizados en `Jarvis.E2ETests` ([HU-050](../09-backlog/stories/EPIC-10-simulators.md)). Setup común: owner O1 creado; devices `esp32-sim` (E) y `windows-sim` (W) registrados; grant `(E, W, OPEN_CHROME)`; ambos simuladores conectados salvo que se indique. Timeouts reducidos por configuración: ACK 3 s, sweeps cada 1 s, presencia Stale 5 s / Offline 10 s.

## E2E-01 — Caso principal

1. E pulsa botón → `task.create OPEN_CHROME` hacia W.
2. **Esperado:** E recibe `request.accepted`; W recibe `task.dispatch` una vez; W ejecuta una vez; E recibe `task.update` `Running` y `Completed` con `result.message = "Chrome opened (simulated)"`.
3. Tarea en BD `Completed` con `delivered_at`, `started_at`, `completed_at`, `source_device_id = E`.
4. Eventos por `correlationId`: `TASK_CREATED`, `TASK_DISPATCHED(1)`, `TASK_DELIVERED`, `TASK_STARTED`, `TASK_COMPLETED`.
5. W recibió `task.update.ack` y marcó la tarea *settled*.
6. Cliente SignalR de O1 recibió las notificaciones `task.*` en orden.

## E2E-02 — Target offline

1. W detenido. E crea tarea con `expiresInSeconds = 120`.
2. **Esperado:** `Pending`, `attempt_count = 0`.
3. Arrancar W → `presence.hello` → la tarea se despacha y completa como E2E-01.

## E2E-03 — ACK perdido y redelivery sin doble ejecución

1. W con `--drop-acks 2` (recibe y persiste pero no publica ack las dos primeras veces).
2. **Esperado:** `TASK_DISPATCHED` con attempt 1, 2, 3; W recibe el dispatch 3 veces; **contador de ejecuciones = 1**; tarea `Completed`.

## E2E-04 — Corte de conexión durante la entrega

1. W recibe `task.dispatch`, persiste, y se corta su conexión TCP antes del ack (flag de prueba).
2. W reconecta con nueva sesión: `presence.hello` con `openTaskIds = [A]`.
3. **Esperado:** el Cloud no revierte A a `Pending`; W reenvía ack desde su outbox; ejecución única; `Completed`. El LWT de la sesión vieja no deja W `Offline`.

## E2E-05 — Autorización

1. Borrar el grant. E intenta crear la tarea → `request.rejected TASK_NOT_GRANTED`, evento `TASK_REJECTED`.
2. E intenta publicar en `jarvis/v1/devices/{W}/up/tasks` y suscribirse a `jarvis/v1/devices/{W}/down/#` → el broker lo impide; W no recibe nada; ningún cambio en BD.
3. E publica un envelope con `deviceId = W` en su propio topic → `SECURITY_VIOLATION topic_device_mismatch`.
4. Owner O2 con token válido consulta la tarea de O1 → 404.

## E2E-06 — Expiración

1. W detenido. E crea tarea con `expiresInSeconds = 10`.
2. **Esperado:** a los ~10 s → `Expired` (`NOT_DELIVERED`), `TASK_EXPIRED`, E recibe `task.update Expired`.
3. Arrancar W → no recibe la tarea.

## E2E-07 — Revocación

1. Revocar E vía REST.
2. **Esperado:** E desconectado en < 2 s; reconexión rechazada; `DEVICE_REVOKED`; grants de E eliminados.
3. Revocar W con una tarea `Running` (W con `--slow 30`) → tarea `Cancelled` `DEVICE_REVOKED`.

## E2E-08 — Presencia

1. Matar el proceso de W sin DISCONNECT → `Offline` (`lwt`) dentro del umbral.
2. Pausar el envío de heartbeats de W sin cortar TCP (flag) → `Stale` y luego `Offline` (`heartbeat_timeout`).
3. W con `busy = true` → `status = Busy`.
4. Deshabilitar W → `status = Disabled`, desconectado.

## E2E-09 — Cancelación

1. W con `--slow 30`. E crea tarea; esperar `Running`.
2. Owner cancela por REST → 202; W recibe `task.cancel`; responde `task.cancelled` → `Cancelled`, E recibe `task.update Cancelled`.
3. Cancelar una tarea `Pending` (W offline) → 200 `Cancelled` inmediato; al conectar W no la recibe.

## E2E-10 — Reinicios de infraestructura

1. W detenido; E crea tarea (`Pending`).
2. Reiniciar el contenedor de la API y el de Mosquitto.
3. Arrancar W.
4. **Esperado:** la tarea se entrega y completa; los simuladores reconectan solos; ningún evento duplicado.
5. Variante: W publica `task.completed` mientras la API está detenida → al volver la API procesa el mensaje desde la sesión persistente y la tarea queda `Completed`.
