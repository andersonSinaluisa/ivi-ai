# EPIC-02 — Identity & Devices

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

---

## HU-008 — Entidad User y comando `bootstrap-owner`

- **Objetivo:** crear owners sin registro público.
- **Descripción:** entidad `User` y tabla `users` ([Database Design](../../04-data/database-design.md#users)); subcomando `jarvis-api bootstrap-owner --email <email> --display-name <name>` que lee la contraseña por stdin (nunca por argumento) y crea el user.
- **Criterios de aceptación:**
  1. Crea un user `Active` con hash `PasswordHasher<T>`.
  2. Email duplicado (case-insensitive) → error y código de salida ≠ 0.
  3. Contraseña < 12 caracteres o en lista de comunes → rechazada.
  4. Registra el evento `USER_CREATED` (actor `System`, `owner_id` = el nuevo user) y un log `Information` sin el email completo.
- **Dependencias:** HU-003, HU-038.
- **Consideraciones técnicas:** la contraseña no debe aparecer en historial de shell ni logs. Lista de contraseñas comunes embebida como recurso.
- **Pruebas esperadas:** integración del comando; unit de política de contraseña.
- **Definition of Done:** DoD común + instrucción de uso en README.

---

## HU-009 — Login y access token JWT

- **Objetivo:** `POST /api/v1/auth/login` según [REST API](../../03-protocols/rest-api.md#post-apiv1authlogin).
- **Descripción:** validar credenciales, emitir JWT HS256 (15 min) con claims definidos en [Security Architecture](../../05-security/security-architecture.md#jwt), lockout tras 5 fallos (15 min), eventos `USER_LOGIN_SUCCEEDED`, `USER_LOGIN_FAILED`, `USER_LOCKED_OUT`. Configurar autenticación JWT en la API. `GET /api/v1/me` para users.
- **Criterios de aceptación:**
  1. Credenciales válidas → 200 con token; token aceptado en `/api/v1/me`.
  2. Email inexistente y contraseña incorrecta → mismo 401 `INVALID_CREDENTIALS` y tiempo similar (se ejecuta un hash ficticio si el user no existe).
  3. 5 fallos → 423 `ACCOUNT_LOCKED` durante 15 min, incluso con contraseña correcta.
  4. User `Disabled` → 401.
  5. Token expirado o con `typ` incorrecto → 401.
- **Dependencias:** HU-008.
- **Consideraciones técnicas:** soporte de clave `current`/`previous`. Skew 30 s.
- **Pruebas esperadas:** integración de todos los criterios; unit del emisor de tokens.
- **Definition of Done:** DoD común.

---

## HU-010 — Refresh tokens con rotación y logout

- **Objetivo:** sesiones largas seguras.
- **Descripción:** tabla `refresh_tokens`; login devuelve refresh token opaco (`jrt_` + 256 bits); `POST /auth/refresh` rota; reutilización de un token ya rotado revoca la familia; `POST /auth/logout` revoca.
- **Criterios de aceptación:**
  1. Refresh válido → nuevos tokens; el anterior deja de funcionar.
  2. Reusar un refresh rotado → 401 y todos los tokens de la familia revocados.
  3. Refresh expirado (30 días) → 401.
  4. Logout idempotente → 204.
  5. Solo se persiste SHA-256 del token.
- **Dependencias:** HU-009.
- **Consideraciones técnicas:** comparación por hash con índice único.
- **Pruebas esperadas:** integración de rotación, reuse detection y expiración (con `TimeProvider` falso).
- **Definition of Done:** DoD común.

---

## HU-011 — Modelo de dominio Device y estado calculado

- **Objetivo:** reglas de Device en el dominio, sin infraestructura.
- **Descripción:** entidad `Device`, `DeviceCredential`, enums, value objects (`DeviceName`, `Capability`, `TaskType`), métodos `Disable`, `Enable`, `Revoke`, `RotateCredential`, `MarkConnected`, `MarkSeen`, `MarkStale`, `MarkDisconnected`, cálculo de `DeviceStatus` según [Device Model](../../02-domain/device-model.md#devicestatus-calculado).
- **Criterios de aceptación:**
  1. Tabla de precedencia de `DeviceStatus` cubierta por tests (todas las combinaciones).
  2. `Revoked` es irreversible (`Enable` lanza error de dominio).
  3. Máximo una credencial `Active`.
  4. Capabilities/supported types validan formato y cardinalidad.
  5. Métodos registran eventos de dominio (`DEVICE_*`).
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** sin atributos de EF; configuración de mapeo en Infrastructure (en HU-012).
- **Pruebas esperadas:** unit tests en `Jarvis.Domain.Tests`.
- **Definition of Done:** DoD común.

---

## HU-012 — Registrar device

- **Objetivo:** `POST /api/v1/devices` según [REST API](../../03-protocols/rest-api.md#post-apiv1devices).
- **Descripción:** tablas `devices` y `device_credentials`; generación de secreto `jdv_…`; hash HMAC con pepper; provisión en dynsec **antes** del commit; respuesta con secreto una sola vez; `DEVICE_REGISTERED`.
- **Criterios de aceptación:**
  1. 201 con `credential.secret` y `Cache-Control: no-store`; el secreto no se puede volver a obtener.
  2. BD guarda solo `secret_hash` (32 bytes) y `secret_hint`.
  3. El device puede conectarse a Mosquitto con `deviceId`/secreto inmediatamente después.
  4. Broker caído → 503 y nada persistido.
  5. Nombre duplicado en el owner → 409; mismo nombre en otro owner → permitido.
  6. Límite de 100 devices por owner → 422.
- **Dependencias:** HU-011, HU-018, HU-043.
- **Consideraciones técnicas:** orden transacción/broker en [Data Flow §1](../../01-architecture/data-flow.md#1-registro-de-un-device).
- **Pruebas esperadas:** integración con PostgreSQL + Mosquitto (Testcontainers), incluida conexión MQTT real del device y caso broker caído.
- **Definition of Done:** DoD común.

---

## HU-013 — Listar y consultar devices

- **Objetivo:** `GET /api/v1/devices` y `GET /api/v1/devices/{id}`.
- **Descripción:** proyección a la representación `Device` con `status` calculado, filtros `type` y `status`, paginación por cursor.
- **Criterios de aceptación:**
  1. Solo devices del owner; device de otro owner → 404.
  2. Nunca se devuelve `secret_hash`.
  3. Filtro por `status` usa el estado calculado.
  4. Principal Device → 403.
- **Dependencias:** HU-012.
- **Consideraciones técnicas:** `AsNoTracking`, keyset `(created_at, id)`.
- **Pruebas esperadas:** integración con dos owners; paginación.
- **Definition of Done:** DoD común.

---

## HU-014 — Renombrar, deshabilitar y habilitar device

- **Objetivo:** `PATCH /devices/{id}`, `POST /devices/{id}/disable`, `POST /devices/{id}/enable`.
- **Descripción:** disable llama `disableClient` en dynsec (si el broker está caído, se confirma en BD y la reconciliación lo aplica), pone `presence_state = Offline`, eventos `DEVICE_UPDATED`/`DEVICE_DISABLED`/`DEVICE_ENABLED`/`DEVICE_DISCONNECTED`.
- **Criterios de aceptación:**
  1. Disable desconecta una sesión MQTT activa del device en < 2 s.
  2. Device deshabilitado no puede reconectar ni obtener device token (403 `DEVICE_DISABLED`).
  3. Operaciones idempotentes sin eventos duplicados.
  4. Operar sobre revocado → 409 `DEVICE_REVOKED`.
- **Dependencias:** HU-012, HU-018.
- **Consideraciones técnicas:** caché de estado del principal invalidada localmente al cambiar.
- **Pruebas esperadas:** integración con Mosquitto real.
- **Definition of Done:** DoD común.

---

## HU-015 — Revocar device

- **Objetivo:** `POST /api/v1/devices/{id}/revoke` irreversible.
- **Descripción:** credenciales `Revoked`, `deleteClient` en dynsec, tareas no terminales como target → `Cancelled` (`DEVICE_REVOKED`), grants eliminados, `DEVICE_REVOKED`.
- **Criterios de aceptación:**
  1. Sesión MQTT cerrada y reconexión rechazada.
  2. Tareas `Pending`/`Dispatched`/`Delivered`/`Running`/`WaitingForApproval` del device quedan `Cancelled` con eventos `TASK_CANCELLED`.
  3. Grants de origen y destino eliminados (`GRANT_DELETED`).
  4. Segunda llamada → 200 sin cambios.
- **Dependencias:** HU-014, HU-024.
- **Consideraciones técnicas:** todo en una transacción; broker después (reconciliación si falla).
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-016 — Rotar credencial de device

- **Objetivo:** `POST /api/v1/devices/{id}/credentials/rotate`.
- **Descripción:** según [Device Authentication — Rotación](../../05-security/device-authentication.md#rotación).
- **Criterios de aceptación:**
  1. Nuevo secreto devuelto una vez; el anterior deja de funcionar en MQTT y REST.
  2. Sesión MQTT activa cerrada.
  3. `DEVICE_CREDENTIAL_ROTATED` con `hint`.
  4. Broker caído → 503 sin cambios en BD.
- **Dependencias:** HU-012, HU-018.
- **Consideraciones técnicas:** verificar empíricamente si `setClientPassword` desconecta; si no, usar disable+enable.
- **Pruebas esperadas:** integración con Mosquitto real.
- **Definition of Done:** DoD común.
