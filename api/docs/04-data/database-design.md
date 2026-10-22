# Database Design (PostgreSQL 17)

Diagrama: [ERD](erd.md). Estrategia: [Persistence Strategy](persistence-strategy.md). JSONB: [ADR-007](../08-decisions/ADR-007-jsonb-usage.md).

## Convenciones

- Nombres `snake_case`, tablas en plural. Esquema `jarvis`.
- PK `id uuid` (UUIDv7 generado por la app), salvo `events.seq`.
- Tiempos `timestamptz`, siempre UTC.
- Enums como `text` + `CHECK` (no tipos `enum` de PostgreSQL: agregar valores con `ALTER TYPE` complica migraciones; `text` + `CHECK` se cambia en una migración simple).
- Extensión `citext` para emails.
- Concurrencia optimista con `xmin` (Npgsql lo mapea como concurrency token) en `devices` y `tasks`.
- Todas las FK `ON DELETE RESTRICT` salvo donde se indica. No se borran users ni devices en MVP (se deshabilitan/revocan).

## Tablas

### `users`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `email` | citext | no | UNIQUE, `CHECK (length(email) <= 254)` |
| `display_name` | text | no | `CHECK (length(display_name) between 1 and 100)` |
| `password_hash` | text | no | |
| `status` | text | no | `'Active'`, `CHECK (status in ('Active','Disabled'))` |
| `failed_login_count` | int | no | `0` |
| `locked_until` | timestamptz | sí | |
| `last_login_at` | timestamptz | sí | |
| `created_at` | timestamptz | no | |
| `updated_at` | timestamptz | no | |

### `refresh_tokens`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `user_id` | uuid | no | FK → `users.id` ON DELETE CASCADE |
| `family_id` | uuid | no | Todos los tokens de una cadena de rotación |
| `token_hash` | bytea | no | UNIQUE. SHA-256 del token |
| `created_at` | timestamptz | no | |
| `expires_at` | timestamptz | no | created + 30 días |
| `revoked_at` | timestamptz | sí | |
| `replaced_by_id` | uuid | sí | FK → `refresh_tokens.id` |

Índices: `ix_refresh_tokens_user_id (user_id)`, `ix_refresh_tokens_family_id (family_id)`.

### `devices`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `owner_id` | uuid | no | FK → `users.id` |
| `name` | text | no | `CHECK (length(name) between 1 and 64)` |
| `type` | text | no | `CHECK (type in ('Windows','ESP32','Mobile','Web','Unknown'))` |
| `admin_state` | text | no | `'Active'`, `CHECK (admin_state in ('Active','Disabled','Revoked'))` |
| `presence_state` | text | no | `'Offline'`, `CHECK (presence_state in ('Online','Stale','Offline'))` |
| `reported_busy` | boolean | no | `false` |
| `current_session_id` | text | sí | `CHECK (length(current_session_id) <= 64)` |
| `capabilities` | text[] | no | `'{}'`, `CHECK (cardinality(capabilities) <= 32)` |
| `supported_task_types` | text[] | no | `'{}'`, `CHECK (cardinality(supported_task_types) <= 64)` |
| `reported_info` | jsonb | no | `'{}'`, `CHECK (jsonb_typeof(reported_info) = 'object')` |
| `last_seen_at` | timestamptz | sí | |
| `last_connected_at` | timestamptz | sí | |
| `last_disconnected_at` | timestamptz | sí | |
| `broker_synced_at` | timestamptz | sí | nulo = pendiente de sincronizar con dynsec |
| `disabled_at` | timestamptz | sí | |
| `revoked_at` | timestamptz | sí | |
| `created_at` | timestamptz | no | |
| `updated_at` | timestamptz | no | |

Constraints / índices:
- `ux_devices_owner_name UNIQUE (owner_id, name)`
- `ix_devices_owner_id (owner_id)` (cubierto por el unique; no crear aparte)
- `ix_devices_presence_sweep (presence_state, last_seen_at) WHERE presence_state <> 'Offline'` — para el PresenceSweeper
- `ix_devices_broker_unsynced (id) WHERE broker_synced_at IS NULL`
- `CHECK (admin_state <> 'Revoked' OR revoked_at IS NOT NULL)`

### `device_credentials`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `device_id` | uuid | no | FK → `devices.id` |
| `kind` | text | no | `'Secret'`, `CHECK (kind in ('Secret','ClientCertificate'))` |
| `secret_hash` | bytea | sí | 32 bytes (HMAC-SHA256) |
| `secret_hint` | text | sí | 4 caracteres |
| `certificate_thumbprint` | text | sí | reservado |
| `status` | text | no | `CHECK (status in ('Active','Revoked'))` |
| `created_at` | timestamptz | no | |
| `expires_at` | timestamptz | sí | reservado (nulo = sin expiración) |
| `revoked_at` | timestamptz | sí | |
| `revoked_reason` | text | sí | `CHECK (length(revoked_reason) <= 200)` |
| `last_used_at` | timestamptz | sí | |

Constraints / índices:
- `ux_device_credentials_active UNIQUE (device_id) WHERE status = 'Active'` — máximo una activa
- `CHECK ((kind = 'Secret' AND secret_hash IS NOT NULL) OR (kind = 'ClientCertificate' AND certificate_thumbprint IS NOT NULL))`

### `device_task_grants`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `owner_id` | uuid | no | FK → `users.id` |
| `source_device_id` | uuid | no | FK → `devices.id` ON DELETE CASCADE |
| `target_device_id` | uuid | no | FK → `devices.id` ON DELETE CASCADE |
| `task_type` | text | no | `CHECK (task_type ~ '^[A-Z][A-Z0-9_]{1,63}$')` |
| `created_by_user_id` | uuid | no | FK → `users.id` |
| `created_at` | timestamptz | no | |

Constraints / índices:
- `ux_grants UNIQUE (source_device_id, target_device_id, task_type)` — también sirve para la consulta de autorización
- `CHECK (source_device_id <> target_device_id)`
- La pertenencia de ambos devices al mismo `owner_id` se valida en Application (una FK compuesta `(owner_id, id)` sería posible pero agrega índices únicos redundantes; se prefiere validación + test).

### `tasks`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `id` | uuid | no | PK |
| `owner_id` | uuid | no | FK → `users.id` |
| `requested_by_type` | text | no | `CHECK (requested_by_type in ('User','Device'))` |
| `requested_by_user_id` | uuid | sí | FK → `users.id` |
| `source_device_id` | uuid | sí | FK → `devices.id` |
| `target_device_id` | uuid | no | FK → `devices.id` |
| `type` | text | no | `CHECK (type ~ '^[A-Z][A-Z0-9_]{1,63}$')` |
| `payload` | jsonb | no | `'{}'`, `CHECK (jsonb_typeof(payload) = 'object')` |
| `status` | text | no | `CHECK (status in ('Pending','Dispatched','Delivered','Running','WaitingForApproval','Completed','Failed','Cancelled','Expired'))` |
| `status_reason` | text | sí | `CHECK (length(status_reason) <= 100)` |
| `correlation_id` | uuid | no | |
| `idempotency_key` | text | sí | `CHECK (length(idempotency_key) between 1 and 100)` |
| `request_hash` | bytea | sí | |
| `attempt_count` | int | no | `0` |
| `max_attempts` | int | no | `5` |
| `expires_at` | timestamptz | no | |
| `max_duration_seconds` | int | sí | `CHECK (max_duration_seconds between 1 and 86400)` |
| `created_at` | timestamptz | no | |
| `dispatched_at` | timestamptz | sí | |
| `delivered_at` | timestamptz | sí | |
| `started_at` | timestamptz | sí | |
| `completed_at` | timestamptz | sí | momento en que alcanzó estado terminal |
| `cancel_requested_at` | timestamptz | sí | |
| `last_update_at` | timestamptz | sí | |
| `progress_percent` | smallint | sí | `CHECK (progress_percent between 0 and 100)` |
| `progress_message` | text | sí | `CHECK (length(progress_message) <= 200)` |
| `result` | jsonb | sí | |
| `error_code` | text | sí | `CHECK (length(error_code) <= 64)` |
| `error_message` | text | sí | `CHECK (length(error_message) <= 500)` |

Constraints:
- `CHECK ((requested_by_type = 'User' AND requested_by_user_id IS NOT NULL AND source_device_id IS NULL) OR (requested_by_type = 'Device' AND source_device_id IS NOT NULL AND requested_by_user_id IS NULL))`
- `CHECK (status NOT IN ('Completed','Failed','Cancelled','Expired') OR completed_at IS NOT NULL)`
- `CHECK (status <> 'Completed' OR result IS NOT NULL)`
- `CHECK (status NOT IN ('Failed','Cancelled','Expired') OR error_code IS NOT NULL)`
- `CHECK (pg_column_size(payload) <= 16384)` y `CHECK (result IS NULL OR pg_column_size(result) <= 32768)` — red de seguridad; el límite real se valida en la app sobre el JSON serializado.

Índices:
| Nombre | Definición | Uso |
|---|---|---|
| `ix_tasks_dispatch` | `(target_device_id, created_at) WHERE status = 'Pending'` | Dispatcher |
| `ix_tasks_ack_timeout` | `(dispatched_at) WHERE status = 'Dispatched'` | DeliveryTimeoutSweeper |
| `ix_tasks_expiry` | `(expires_at) WHERE status IN ('Pending','Dispatched')` | TaskExpirySweeper |
| `ix_tasks_active_by_target` | `(target_device_id) WHERE status IN ('Dispatched','Delivered','Running','WaitingForApproval')` | Reconciliación en hello, timeouts de ejecución, revocación |
| `ix_tasks_owner_created` | `(owner_id, created_at DESC, id DESC)` | Listado y paginación |
| `ix_tasks_source_created` | `(source_device_id, created_at DESC) WHERE source_device_id IS NOT NULL` | Listado para devices |
| `ix_tasks_correlation` | `(correlation_id)` | Trazabilidad |
| `ux_tasks_idempotency` | `UNIQUE (owner_id, requested_by_type, requested_by_user_id, source_device_id, idempotency_key) NULLS NOT DISTINCT WHERE idempotency_key IS NOT NULL` (equivalente al `COALESCE` y expresable en EF Core) | Idempotencia |

### `events`

| Columna | Tipo | Null | Default / Constraint |
|---|---|---|---|
| `seq` | bigint | no | `GENERATED ALWAYS AS IDENTITY`, PK |
| `id` | uuid | no | UNIQUE |
| `owner_id` | uuid | sí | FK → `users.id` |
| `type` | text | no | `CHECK (type ~ '^[A-Z][A-Z0-9_]{2,63}$')` |
| `occurred_at` | timestamptz | no | |
| `recorded_at` | timestamptz | no | `now()` |
| `correlation_id` | uuid | no | |
| `actor_type` | text | no | `CHECK (actor_type in ('User','Device','System'))` |
| `actor_id` | uuid | sí | |
| `device_id` | uuid | sí | sin FK (el log sobrevive a cambios; la integridad se garantiza al escribir) |
| `task_id` | uuid | sí | sin FK (idem) |
| `data` | jsonb | no | `'{}'`, `CHECK (pg_column_size(data) <= 4096)` |

Índices:
- `ix_events_owner_seq (owner_id, seq)` — catch-up y listado
- `ix_events_task (task_id, seq) WHERE task_id IS NOT NULL`
- `ix_events_device (device_id, seq) WHERE device_id IS NOT NULL`
- `ix_events_correlation (correlation_id)`
- `ix_events_recorded_at (recorded_at)` — retención

Nota sobre `seq` (riesgo de huecos en catch-up): con transacciones concurrentes, un `seq` menor puede confirmarse **después** que uno mayor; un cliente que ya leyó el mayor y pide `after` lo saltaría. Alternativas: filtrar eventos recientes por tiempo (frágil) o usar `pg_snapshot_xmin` (complejo). **Decisión MVP:** las transacciones que insertan eventos toman `pg_advisory_xact_lock(hashtextextended(owner_id::text, 0))` antes de insertar, serializando la escritura de eventos por owner. Así el orden de `seq` coincide con el orden de commit dentro de un owner, y los clientes solo leen eventos de su owner. Costo despreciable con el volumen esperado; revisar si un owner supera ~100 eventos/s. Los eventos sin owner (`owner_id` nulo) no se exponen por catch-up.

## Roles de base de datos

| Rol | Permisos | Usado por |
|---|---|---|
| `jarvis_owner` | Dueño del esquema | Nadie en runtime; solo administración manual |
| `jarvis_migrator` | DDL sobre `jarvis` | Paso de migraciones al desplegar (`jarvis-api migrate`) |
| `jarvis_app` | `SELECT, INSERT, UPDATE, DELETE` en todas las tablas **excepto** `events` (`SELECT, INSERT` solamente). `DELETE` solo en `refresh_tokens` y `device_task_grants`. | API en runtime |
| `jarvis_maintenance` | `DELETE` en `events`, `refresh_tokens` | Job de retención |
| `jarvis_backup` | `SELECT` en todo (o rol con `pg_read_all_data`) | `pg_dump` |

`users`, `devices`, `device_credentials` y `tasks` no admiten `DELETE` desde `jarvis_app`: la aplicación nunca borra estos datos.

## Volumen estimado

| Tabla | Crecimiento | 1 año |
|---|---|---|
| `tasks` | ~500/día | ~180 K filas, < 200 MB |
| `events` | ~5 000/día | ~1.8 M filas, < 1 GB (retención 180 días) |
| Resto | Despreciable | |

Holgado para 100 GB de disco.
