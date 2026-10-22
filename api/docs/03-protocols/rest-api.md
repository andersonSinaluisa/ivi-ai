# REST API — v1

## Convenciones

| Tema | Decisión |
|---|---|
| Base | `https://${JARVIS_DOMAIN}/api/v1` |
| Versionado | En la ruta (`/api/v1`). Cambios compatibles (campos nuevos opcionales, endpoints nuevos) no cambian versión. Un cambio incompatible crea `/api/v2` y `/api/v1` se mantiene al menos 6 meses. |
| Formato | JSON UTF-8, camelCase, enums como string, fechas ISO-8601 UTC. |
| IDs | UUID (v7) en minúsculas. |
| Errores | RFC 9457 Problem Details (`application/problem+json`). Ver [Error Model](error-model.md). |
| Autenticación | `Authorization: Bearer <JWT>`. Dos tipos de principal: **User** (`typ=user`) y **Device** (`typ=device`). Ver [Authorization](../05-security/authorization.md). |
| Correlación | Header opcional `X-Correlation-Id` (UUID). Si falta, se genera. Siempre se devuelve en la respuesta. |
| Paginación | Por cursor: `?limit=50&cursor=<opaco>`. Respuesta `{ "items": [...], "nextCursor": "..." \| null }`. `limit` máx. 200. |
| Recursos ajenos | Un recurso de otro owner responde **404**, nunca 403 (no revela existencia). |
| Idempotencia | `POST /tasks` acepta `Idempotency-Key`. Las acciones (`/disable`, `/enable`, `/revoke`, `/cancel`) son idempotentes por diseño. |
| Rate limiting | Ver [Security Architecture](../05-security/security-architecture.md#rate-limiting). Exceso → `429` con `Retry-After`. |

Sobre la ruta propuesta `POST /api/devices/register`: se reemplaza por `POST /api/v1/devices` (creación de recurso estándar). El registro **siempre** lo hace un owner autenticado; no existe auto-registro anónimo de devices (sería una puerta abierta). El emparejamiento por código para ESP32 es futuro ([ESP32 Contract](../07-integrations/esp32-contract.md)).

## Resumen de endpoints

| Método | Ruta | Principal | Propósito |
|---|---|---|---|
| POST | `/api/v1/auth/login` | Anónimo | Login de user |
| POST | `/api/v1/auth/refresh` | Anónimo (refresh token) | Renovar tokens |
| POST | `/api/v1/auth/logout` | User | Revocar refresh token |
| POST | `/api/v1/auth/device-token` | Anónimo (credencial device) | Token de device |
| GET | `/api/v1/me` | User, Device | Identidad actual |
| POST | `/api/v1/devices` | User | Registrar device |
| GET | `/api/v1/devices` | User | Listar devices |
| GET | `/api/v1/devices/{id}` | User | Detalle |
| PATCH | `/api/v1/devices/{id}` | User | Renombrar |
| POST | `/api/v1/devices/{id}/disable` | User | Deshabilitar |
| POST | `/api/v1/devices/{id}/enable` | User | Rehabilitar |
| POST | `/api/v1/devices/{id}/revoke` | User | Revocar permanentemente |
| POST | `/api/v1/devices/{id}/credentials/rotate` | User | Rotar credencial |
| GET | `/api/v1/devices/{id}/grants` | User | Listar grants del device origen |
| POST | `/api/v1/devices/{id}/grants` | User | Crear grant |
| DELETE | `/api/v1/devices/{id}/grants/{grantId}` | User | Eliminar grant |
| POST | `/api/v1/tasks` | User, Device | Crear tarea |
| GET | `/api/v1/tasks` | User, Device | Listar tareas |
| GET | `/api/v1/tasks/{id}` | User, Device | Detalle |
| POST | `/api/v1/tasks/{id}/approval` | User, Device | Decidir en remoto una aprobación (firmado) |
| POST | `/api/v1/tasks/{id}/cancel` | User, Device | Cancelar |
| GET | `/api/v1/events` | User | Consultar event log |
| GET | `/health/live` | Anónimo | Liveness |
| GET | `/health/ready` | Solo red interna | Readiness |

**No existe** ningún endpoint de ejecución genérica (`/execute`, `/shell`, `/command`). Añadir uno viola [ADR-010](../08-decisions/ADR-010-llm-independence.md) y el principio de separación transporte/ejecución.

---

## Auth

### POST /api/v1/auth/login

- **Propósito:** autenticar a un user.
- **Request:**
  ```json
  { "email": "owner@example.com", "password": "correct horse battery staple" }
  ```
- **Response 200:**
  ```json
  {
    "accessToken": "eyJ...",
    "accessTokenExpiresAt": "2026-09-19T15:15:00Z",
    "refreshToken": "jrt_3q2+...",
    "refreshTokenExpiresAt": "2026-10-19T15:00:00Z",
    "user": { "id": "0192...", "email": "owner@example.com", "displayName": "Owner" }
  }
  ```
- **Errores:** `400 VALIDATION_FAILED`, `401 INVALID_CREDENTIALS` (mismo mensaje para email inexistente y contraseña incorrecta; tiempo de respuesta constante), `423 ACCOUNT_LOCKED`, `429 RATE_LIMITED`.
- **Idempotencia:** no aplica. Eventos: `USER_LOGIN_SUCCEEDED` / `USER_LOGIN_FAILED` / `USER_LOCKED_OUT`.

### POST /api/v1/auth/refresh

- **Request:** `{ "refreshToken": "jrt_..." }`
- **Response 200:** mismo cuerpo que login. El refresh token usado queda revocado (rotación).
- **Errores:** `401 INVALID_REFRESH_TOKEN`. Si se presenta un refresh token **ya rotado**, se revoca toda la familia (detección de robo) y responde 401.

### POST /api/v1/auth/logout

- **Auth:** User. **Request:** `{ "refreshToken": "jrt_..." }`. **Response:** `204`. Idempotente.

### POST /api/v1/auth/device-token

- **Propósito:** que un device obtenga un JWT para usar la REST API.
- **Request:**
  ```json
  { "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002", "secret": "jdv_Zm9vYmFy..." }
  ```
- **Response 200:**
  ```json
  { "accessToken": "eyJ...", "accessTokenExpiresAt": "2026-09-19T15:10:00Z" }
  ```
- **Errores:** `401 INVALID_CREDENTIALS` (device inexistente, secreto incorrecto, credencial revocada), `403 DEVICE_DISABLED`, `429`.
- **Notas:** comparación en tiempo constante. Actualiza `device_credentials.last_used_at` (como máximo una vez por minuto). Fallos → `DEVICE_AUTH_FAILED`.

### GET /api/v1/me

- **Response 200 (User):** `{ "principalType": "User", "id": "...", "ownerId": "...", "email": "...", "displayName": "..." }`
- **Response 200 (Device):** `{ "principalType": "Device", "id": "...", "ownerId": "...", "name": "...", "type": "Windows" }`

---

## Devices

### Representación `Device`

```json
{
  "id": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "name": "PC Escritorio",
  "type": "Windows",
  "status": "Online",
  "adminState": "Active",
  "presenceState": "Online",
  "busy": false,
  "capabilities": ["agent", "windows-control", "notifications"],
  "supportedTaskTypes": ["OPEN_CHROME"],
  "reportedInfo": { "softwareVersion": "windows-simulator/0.1.0", "protocolVersion": "1.0", "platform": "windows" },
  "lastSeenAt": "2026-09-19T15:03:59Z",
  "lastConnectedAt": "2026-09-19T15:00:00Z",
  "lastDisconnectedAt": null,
  "brokerSynced": true,
  "credential": { "id": "0192...", "hint": "x7Qa", "createdAt": "2026-09-01T10:00:00Z", "lastUsedAt": "2026-09-19T15:00:00Z" },
  "createdAt": "2026-09-01T10:00:00Z",
  "updatedAt": "2026-09-19T15:00:00Z"
}
```

`brokerSynced = false` indica que el client MQTT del device no existe en el broker (p. ej. volumen de Mosquitto perdido) y el owner debe rotar la credencial ([Persistence Strategy](../04-data/persistence-strategy.md#reconciliación-de-dynsec-sin-secretos-en-claro)).

### POST /api/v1/devices

- **Propósito:** registrar un device y emitir su credencial.
- **Auth:** User. **Autorización:** el device queda con `ownerId` = user actual.
- **Request:**
  ```json
  { "name": "PC Escritorio", "type": "Windows" }
  ```
- **Response 201** (`Location: /api/v1/devices/{id}`, `Cache-Control: no-store`):
  ```json
  {
    "device": { "...": "representación Device" },
    "credential": {
      "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
      "secret": "jdv_Zm9vYmFyYmF6cXV4Li4u",
      "mqtt": { "host": "${JARVIS_DOMAIN}", "port": 8883, "username": "0192a6f0-2222-7aaa-8bbb-000000000002", "clientId": "0192a6f0-2222-7aaa-8bbb-000000000002" },
      "warning": "This secret is shown only once. Store it securely."
    }
  }
  ```
- **Errores:** `400 VALIDATION_FAILED` (nombre 1-64 caracteres, tipo válido), `409 DEVICE_NAME_TAKEN`, `422 DEVICE_LIMIT_REACHED` (máx. 100 devices por owner en MVP).
- **Idempotencia:** no idempotente; un reintento crea otro device (el nombre duplicado lo impide con 409).
- **Efectos:** `DEVICE_REGISTERED`; provisión en dynsec **dentro** del flujo (antes del commit). Si el broker no responde en 5 s → rollback y `503 SERVICE_UNAVAILABLE`; no queda nada creado. Si el commit falla después de crear el client en dynsec, la reconciliación elimina el client huérfano.

### GET /api/v1/devices

- **Auth:** User. Filtros: `?type=Windows&status=Online&limit=50&cursor=...`. Orden: `createdAt` ascendente.
- **Response 200:** `{ "items": [Device], "nextCursor": null }`. Nunca incluye secretos.

### GET /api/v1/devices/{id}

- **Auth:** User. **Response 200:** Device. **Errores:** `404 NOT_FOUND`.

### PATCH /api/v1/devices/{id}

- **Request:** `{ "name": "PC Oficina" }` (único campo editable). **Response 200:** Device.
- **Errores:** `400`, `404`, `409 DEVICE_NAME_TAKEN`, `409 DEVICE_REVOKED`. Evento `DEVICE_UPDATED`.

### POST /api/v1/devices/{id}/disable · /enable

- **Response 200:** Device. Idempotentes: deshabilitar un device ya deshabilitado devuelve 200 sin evento nuevo.
- **Errores:** `404`, `409 DEVICE_REVOKED`.
- **Efectos disable:** dynsec `disableClient` (desconexión inmediata), `presence_state = Offline`, `DEVICE_DISABLED`.

### POST /api/v1/devices/{id}/revoke

- **Request:** `{ "reason": "Device lost" }` (opcional, ≤ 200).
- **Response 200:** Device con `adminState = Revoked`. Idempotente.
- **Efectos:** credenciales → `Revoked`; dynsec `deleteClient`; tareas no terminales como target → `Cancelled` (`DEVICE_REVOKED`); grants que lo involucran eliminados; `DEVICE_REVOKED`. **Irreversible.**

### POST /api/v1/devices/{id}/credentials/rotate

- **Response 200** (`Cache-Control: no-store`): mismo bloque `credential` que en el registro.
- **Errores:** `404`, `409 DEVICE_REVOKED`, `503 SERVICE_UNAVAILABLE` (broker no disponible; no se rota nada).
- **Efectos:** credencial anterior `Revoked` inmediatamente; dynsec `setClientPassword` y desconexión del cliente; `DEVICE_CREDENTIAL_ROTATED`. No idempotente (cada llamada emite un secreto nuevo).

### Grants

`GET /api/v1/devices/{id}/grants` → `{ "items": [Grant] }` donde `{id}` es el device **origen**.

`POST /api/v1/devices/{id}/grants`
```json
{ "targetDeviceId": "0192a6f0-2222-7aaa-8bbb-000000000002", "taskType": "OPEN_CHROME" }
```
Response 201:
```json
{ "id": "0192...", "sourceDeviceId": "0192...0001", "targetDeviceId": "0192...0002", "taskType": "OPEN_CHROME", "createdAt": "..." }
```
Errores: `400` (taskType inválido o `*`), `404` (origen o destino no existen para el owner), `409 GRANT_ALREADY_EXISTS`, `422 SAME_DEVICE` (origen = destino).

`DELETE /api/v1/devices/{id}/grants/{grantId}` → `204`. Idempotente (404 solo si el device no existe).

---

## Tasks

### Representación `Task`

```json
{
  "id": "0192a6f4-9000-7c3a-9d2e-00000000a001",
  "type": "OPEN_CHROME",
  "status": "Completed",
  "payload": { "url": "https://example.com" },
  "requestedBy": { "type": "Device", "id": "0192a6f0-1111-7aaa-8bbb-000000000001" },
  "sourceDeviceId": "0192a6f0-1111-7aaa-8bbb-000000000001",
  "targetDeviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
  "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "idempotencyKey": "btn1-000042",
  "attemptCount": 1,
  "expiresAt": "2026-09-19T15:09:05Z",
  "maxDurationSeconds": 120,
  "createdAt": "2026-09-19T15:04:05Z",
  "dispatchedAt": "2026-09-19T15:04:05Z",
  "deliveredAt": "2026-09-19T15:04:05Z",
  "startedAt": "2026-09-19T15:04:06Z",
  "completedAt": "2026-09-19T15:04:07Z",
  "cancelRequestedAt": null,
  "progress": null,
  "result": { "outcome": "SUCCESS", "message": "Chrome opened (simulated)" },
  "error": null
}
```

`progress` = `{ "percent": 40, "message": "..." }` o `null`. `error` = `{ "code": "...", "message": "..." }` o `null`.

### POST /api/v1/tasks

- **Propósito:** crear una tarea para un device destino.
- **Auth:** User o Device.
- **Autorización:**
  - User: el target debe pertenecer al user.
  - Device: debe existir grant `(self, targetDeviceId, type)`. Sin grant → `403 TASK_NOT_GRANTED` + evento `TASK_REJECTED`.
- **Headers:** `Idempotency-Key` (opcional para User, **obligatorio** para Device; 1-100 caracteres `[A-Za-z0-9_.:-]`).
- **Request:**
  ```json
  {
    "targetDeviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
    "type": "OPEN_CHROME",
    "payload": { "url": "https://example.com" },
    "expiresInSeconds": 300,
    "maxDurationSeconds": 120,
    "correlationId": null
  }
  ```
- **Creación firmada** (opcional, [ADR-012](../08-decisions/ADR-012-end-to-end-task-signing.md)): si se envía `signature`, hay que enviar además `taskId` y `expiresAt`, porque forman parte de la cadena que el cliente firmó y el Cloud no puede generarlos sin invalidarla.
  ```json
  {
    "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001",
    "targetDeviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
    "type": "CLOSE_APP",
    "payload": { "appId": "notepad", "force": true },
    "expiresAt": "2026-09-23T10:00:00.000Z",
    "signature": { "alg": "Ed25519", "keyId": "owner-laptop-1", "value": "<64 bytes en base64>" }
  }
  ```
  El Cloud **no verifica la firma** (no tiene la clave pública): comprueba su forma y la retransmite al device, que es quien la valida. Qué se firma exactamente está en [MQTT §12](mqtt-protocol.md#12-firma-end-to-end-adr-012).
- **Validaciones:** `type` con formato `TaskType`; `payload` objeto JSON ≤ 16 KiB (default `{}`); `expiresInSeconds` 10..604800 (default 600); `expiresAt` absoluto dentro de esa misma ventana y excluyente con `expiresInSeconds`; `maxDurationSeconds` 1..86400 u omitido; target existe, pertenece al owner, `adminState = Active`; `type ∈ target.supportedTaskTypes` (si el target nunca envió hello → `422 TASK_TYPE_NOT_SUPPORTED`). Con `signature`: `alg = "Ed25519"`, `keyId` 1-64 `[A-Za-z0-9_.:-]`, `value` base64 de 64 bytes, y `taskId` + `expiresAt` presentes. Sin `signature` no se acepta `taskId`.
- **Response 201** (`Location: /api/v1/tasks/{id}`): Task con `status = Pending`.
- **Response 200:** reintento con la misma `Idempotency-Key` y mismo contenido → la tarea original (header `Idempotent-Replayed: true`).
- **Errores:** `400 VALIDATION_FAILED`, `403 TASK_NOT_GRANTED`, `404 NOT_FOUND` (target), `409 TARGET_DEVICE_UNAVAILABLE` (disabled/revoked), `409 TASK_ID_IN_USE` (el `taskId` firmado ya existe), `413 PAYLOAD_TOO_LARGE`, `422 IDEMPOTENCY_KEY_REUSED` (misma key, contenido distinto), `422 TASK_TYPE_NOT_SUPPORTED`, `429`.
- **Efectos:** `TASK_CREATED`; señal al dispatcher.

### GET /api/v1/tasks

- **Auth:** User (tareas de su owner) o Device (tareas donde es origen **o** target).
- **Filtros:** `status` (múltiple: `?status=Pending&status=Running`), `targetDeviceId`, `sourceDeviceId`, `type`, `correlationId`, `createdFrom`, `createdTo`, `limit`, `cursor`. Orden: `createdAt` descendente.
- **Response 200:** `{ "items": [Task sin payload ni result], "nextCursor": "..." }` (listado liviano; el detalle trae `payload` y `result`).

### GET /api/v1/tasks/{id}

- **Auth:** User owner, o Device origen/target. **Response 200:** Task. **Errores:** `404`.

### POST /api/v1/tasks/{id}/approval

- **Propósito:** transmitir al device la decisión del owner sobre una tarea en `WaitingForApproval` (HU-057).
- **Auth:** User owner, o Device del owner.
- **Request:**
  ```json
  {
    "decision": "GRANTED",
    "nonce": "b6d0f1a2c3e44f5a",
    "decidedAt": "2026-09-23T10:01:30.500Z",
    "signature": { "alg": "Ed25519", "keyId": "owner-laptop-1", "value": "<64 bytes en base64>" }
  }
  ```
- **Validaciones:** `decision` ∈ `GRANTED|DENIED`; `nonce` 1-128 `[A-Za-z0-9_.:-]`; `decidedAt` dentro de ±5 minutos; `signature` obligatoria y bien formada; la tarea existe, es del owner y está en `WaitingForApproval`.
- **Response 202:** el registro de la decisión (`TaskApproval`). **202 significa "transmitida", no "aprobada"**: el device vuelve a verificar firma, nonce y frescura, y puede descartarla. El estado real llega después por el ciclo de vida normal de la tarea.
- **Errores:** `400 VALIDATION_FAILED` (incluida una decisión sin firma o fuera de su ventana), `404`, `409 INVALID_STATE_TRANSITION` (la tarea no está esperando aprobación), `409 APPROVAL_ALREADY_DECIDED`, `409 NONCE_REUSED`.
- **Efectos:** evento `APPROVAL_DECISION_SENT` y mensaje `task.approval_decision` al device. El `APPROVAL_GRANTED`/`APPROVAL_DENIED` lo emite el device **si** aplica la decisión.

### POST /api/v1/tasks/{id}/cancel

- **Auth:** User owner, o Device **origen**.
- **Request:** `{ "reason": "Changed my mind" }` (opcional).
- **Response 200:** Task en `Cancelled` (estaba `Pending`; cancelación inmediata).
- **Response 202:** Task en su estado actual con `cancelRequestedAt` fijado; se envió `task.cancel` al device y el estado final llegará de forma asíncrona.
- **Errores:** `404`, `409 INVALID_STATE_TRANSITION` (terminal).
- **Idempotencia:** repetir sobre una tarea con `cancelRequestedAt` ya fijado devuelve el estado actual sin reenviar evento.

---

## Events

### GET /api/v1/events

- **Auth:** User (solo eventos de su owner).
- **Filtros:** `after` (seq, exclusivo), `type` (múltiple), `deviceId`, `taskId`, `correlationId`, `from`, `to`, `limit` (≤ 500).
- **Orden:** `seq` ascendente cuando se usa `after` (catch-up); descendente en otro caso.
- **Response 200:**
  ```json
  {
    "items": [
      {
        "seq": 1042,
        "id": "0192...",
        "type": "TASK_COMPLETED",
        "occurredAt": "2026-09-19T15:04:06.950Z",
        "recordedAt": "2026-09-19T15:04:07.010Z",
        "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
        "actor": { "type": "Device", "id": "0192a6f0-2222-7aaa-8bbb-000000000002" },
        "deviceId": "0192a6f0-2222-7aaa-8bbb-000000000002",
        "taskId": "0192a6f4-9000-7c3a-9d2e-00000000a001",
        "data": { "resultSizeBytes": 58 }
      }
    ],
    "lastSeq": 1042,
    "hasMore": false
  }
  ```

---

## Health

| Ruta | Expuesta | Respuesta |
|---|---|---|
| `GET /health/live` | Sí (nginx) | `200 {"status":"Healthy"}` si el proceso responde. No consulta dependencias. |
| `GET /health/ready` | No (nginx devuelve 404; accesible en la red interna) | `200 Healthy/Degraded` o `503 Unhealthy`. Detalle en [Observability](../06-infrastructure/observability.md#health-checks). |

## Futuro (no MVP)

| Endpoint | Fase |
|---|---|
| `POST /api/v1/devices/pairing-codes` | Phase 6 |
| `POST /api/v1/devices/me/credentials/rotate` (rotación iniciada por el device) | Phase 2 |
