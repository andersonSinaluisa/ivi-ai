# Persistence Strategy

## Fuente de verdad

| Dato | Fuente de verdad | Copias / derivados |
|---|---|---|
| Users, devices, credenciales, grants, tasks, events | PostgreSQL | — |
| Credenciales MQTT | PostgreSQL (hash HMAC) | Mosquitto dynsec (hash PBKDF2 propio) — **derivado**; reconstruible salvo el secreto (requiere backup del volumen o rotación) |
| Presencia | PostgreSQL (`presence_state`, `last_seen_at`) | Conexiones vivas en el broker |
| Tareas en tránsito | PostgreSQL (`status`) | Mensajes QoS 1 en el broker |
| Notificaciones realtime | PostgreSQL (`events`) | Mensajes SignalR |

Regla: **si el broker, SignalR o la memoria de la API se pierden por completo, el sistema se recupera solo a partir de PostgreSQL.** Única excepción: las contraseñas de dynsec (ver abajo). Ver [ADR-002](../08-decisions/ADR-002-postgresql-source-of-truth.md).

## Escenarios de fallo

| Fallo | Qué se pierde | Recuperación |
|---|---|---|
| Reinicio de la API | Conexiones SignalR, señales en memoria | Sweepers y dispatcher retoman desde BD. Mensajes `up` se quedaron en la sesión persistente del cloud client y se reentregan (el PUBACK solo se envía tras commit). |
| Reinicio de Mosquitto | Sesiones de devices (clean start) y posiblemente mensajes en vuelo | Devices reconectan y mandan `hello`; reconciliación + ACK timeout redespachan. El cloud client reconecta. Dynsec persiste en su volumen (ver siguiente fila si se pierde). |
| Pérdida del volumen de Mosquitto | Config dynsec, sesión persistente del cloud client | Restaurar el volumen desde backup. Si no hay backup: la reconciliación recrea roles y el client `jarvis-cloud`, pero los clients de devices requieren **rotación de credencial** (ver abajo). |
| Reinicio de PostgreSQL | Nada confirmado | La API falla readiness; los background services y el procesador MQTT reintentan en su siguiente ciclo (no se usa `EnableRetryOnFailure` porque el DbContext abre transacciones propias para eventos + advisory lock). Un mensaje MQTT no se confirma hasta el commit, así que se reentrega. |
| Pérdida de PostgreSQL | Todo desde el último backup | Restore ([Backup & Recovery](../06-infrastructure/backup-recovery.md)). |
| Reinicio del VPS | Nada confirmado | `restart: unless-stopped` en todos los servicios; orden por health checks. |

### Reconciliación de dynsec sin secretos en claro

PostgreSQL guarda `HMAC(pepper, secret)`, que no permite recuperar el secreto; dynsec necesita el secreto (o su propio hash PBKDF2) para crear el client. Por eso:

1. Al crear/rotar, la API envía el secreto a dynsec en el mismo request (una vez) y marca `broker_synced_at`.
2. La reconciliación **periódica** solo corrige lo que puede corregir sin secreto: clients sobrantes (se eliminan), estado disabled/enabled, roles. Si un device activo **no existe** en dynsec (volumen perdido), no se puede recrear con el mismo secreto: se marca `broker_synced_at = null`, se registra `Warning` y el owner debe **rotar la credencial** de ese device.
3. Para evitar ese escenario, el volumen `mosquitto-data` se incluye en el backup diario.

Decisión alternativa evaluada: guardar en PostgreSQL el hash PBKDF2 en el formato de dynsec y reinyectarlo (dynsec acepta `encoded_password` al importar la configuración). Es viable y elimina la necesidad de rotar tras pérdida del volumen, pero acopla el esquema al formato interno de Mosquitto. **Se pospone**; se revisará si la pérdida del volumen resulta frecuente.

## Acceso con EF Core

| Operación | Técnica |
|---|---|
| CRUD de devices, grants, users | EF Core tracking normal, una transacción por caso de uso |
| Transiciones de tarea desde MQTT | Cargar la tarea, aplicar método de dominio, `SaveChanges` con `xmin` como concurrency token; en conflicto recargar y reevaluar (máx. 3 intentos) |
| Claim del dispatcher | SQL explícito: `SELECT ... FROM tasks t JOIN devices d ON d.id = t.target_device_id WHERE t.status = 'Pending' AND d.presence_state = 'Online' AND d.admin_state = 'Active' AND t.expires_at > now() ORDER BY t.created_at LIMIT 50 FOR UPDATE OF t SKIP LOCKED` y `UPDATE` a `Dispatched` en la misma transacción |
| Sweepers | `ExecuteUpdate` condicional por lotes, más inserción de eventos. Los que requieren publicar MQTT (timeout de ejecución) procesan fila a fila. |
| Listados | `AsNoTracking`, proyección directa al DTO, paginación keyset por `(created_at, id)` |
| Eventos | Insert en la misma transacción que el cambio (patrón `DbContext.SaveChanges` intercepta los `DomainEvents` de las entidades y agrega filas `events`) |

## Transacción + publicación (sin outbox dedicado)

```text
BEGIN
  cambio de estado (tasks/devices)
  INSERT events
COMMIT
→ publicar MQTT (si aplica)
→ notificar SignalR
```

¿Qué pasa si la API cae entre COMMIT y la publicación?

| Publicación perdida | Recuperación |
|---|---|
| `task.dispatch` | La tarea está `Dispatched` sin ACK → ACK timeout → `Pending` → redispatch. |
| `task.update.ack` | El device reenvía el resultado → estado terminal → se reenvía el ack. |
| `task.cancel` | No hay reenvío implícito por estado, así que se agrega uno explícito: el dispatcher reenvía `task.cancel` para tareas con `cancel_requested_at` no nulo y estado no terminal cada 60 s. |
| `task.update` al origen | Best effort; el origen usa `task.get`. |
| `request.accepted` | El device reintenta `task.create` con la misma `idempotencyKey` y recibe `duplicate: true`. |
| Notificación SignalR | Catch-up por `events`. |

Por eso no se necesita una tabla outbox genérica en el MVP.

## Migraciones

- EF Core migrations en `Jarvis.Infrastructure`.
- Se aplican con un paso explícito de despliegue (`jarvis-api migrate` con el rol `jarvis_migrator`), **no** automáticamente al iniciar la API (evita que el rol de runtime necesite DDL y que varias instancias migren a la vez).
- Toda migración debe ser compatible hacia atrás con la versión anterior de la API durante el despliegue (expand → migrate → contract).

## Retención

| Dato | Retención | Mecanismo |
|---|---|---|
| `events` | 180 días | Job diario, rol `jarvis_maintenance`, borrado por lotes de 5 000 |
| `refresh_tokens` expirados/revocados | 30 días después de expirar | Mismo job |
| `tasks` | Indefinida en MVP | Revisar a 1 M filas (archivado) |
