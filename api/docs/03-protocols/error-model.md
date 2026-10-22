# Error Model

Un único catálogo de códigos para REST, MQTT y errores de tareas.

## REST: RFC 9457 Problem Details

RFC 9457 reemplaza a RFC 7807 con el mismo formato. Content-Type `application/problem+json`.

```json
{
  "type": "urn:jarvis:problem:task-not-granted",
  "title": "Task not granted",
  "status": 403,
  "detail": "Device is not allowed to create OPEN_CHROME tasks for the target device.",
  "instance": "/api/v1/tasks",
  "code": "TASK_NOT_GRANTED",
  "correlationId": "0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
  "traceId": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
}
```

Validación (400):

```json
{
  "type": "urn:jarvis:problem:validation-failed",
  "title": "Validation failed",
  "status": 400,
  "code": "VALIDATION_FAILED",
  "correlationId": "…",
  "traceId": "…",
  "errors": {
    "type": ["Must match ^[A-Z][A-Z0-9_]{1,63}$."],
    "expiresInSeconds": ["Must be between 10 and 604800."]
  }
}
```

Reglas:
- `type` es un URN estable `urn:jarvis:problem:{code-en-kebab-case}` (no depende de dominio).
- `code` es la clave que los clientes deben usar programáticamente.
- `detail` nunca incluye secretos, SQL, stack traces ni datos de otros owners.
- Errores 500: `detail` genérico; el detalle solo en logs con el mismo `correlationId`.

## MQTT: objeto `error`

En `request.rejected`, `task.rejected` y `task.failed`:

```json
{ "code": "TASK_NOT_GRANTED", "message": "Human readable, no secrets", "retryable": false }
```

## Catálogo — errores de API y requests

| Code | HTTP | MQTT | Significado |
|---|---|---|---|
| `VALIDATION_FAILED` | 400 | ✓ | Forma o valores inválidos. |
| `UNAUTHENTICATED` | 401 | — | Falta token o es inválido/expirado. |
| `INVALID_CREDENTIALS` | 401 | — | Login o device-token fallido. |
| `INVALID_REFRESH_TOKEN` | 401 | — | Refresh token inválido, expirado o reutilizado. |
| `FORBIDDEN` | 403 | ✓ | Principal sin permiso para la operación (p. ej. device llamando endpoint de user). |
| `TASK_NOT_GRANTED` | 403 | ✓ | Device sin grant para ese target/tipo. |
| `DEVICE_DISABLED` | 403 | — | Device deshabilitado intentando autenticarse. |
| `NOT_FOUND` | 404 | ✓ | No existe o pertenece a otro owner. |
| `DEVICE_NAME_TAKEN` | 409 | — | Nombre duplicado para el owner. |
| `DEVICE_REVOKED` | 409 | — | Operación sobre device revocado. |
| `GRANT_ALREADY_EXISTS` | 409 | — | |
| `TARGET_DEVICE_UNAVAILABLE` | 409 | ✓ | Target disabled o revoked. |
| `INVALID_STATE_TRANSITION` | 409 | ✓ | Operación no válida en el estado actual de la tarea. |
| `PAYLOAD_TOO_LARGE` | 413 | ✓ | `payload` > 16 KiB o cuerpo > límite. |
| `IDEMPOTENCY_KEY_REUSED` | 422 | ✓ | Misma key con contenido distinto. |
| `IDEMPOTENCY_KEY_REQUIRED` | 400 | ✓ | Device sin `Idempotency-Key`/`idempotencyKey`. |
| `TASK_TYPE_NOT_SUPPORTED` | 422 | ✓ | Target no declara el tipo en `supportedTaskTypes`. |
| `SAME_DEVICE` | 422 | — | Grant con origen = destino. |
| `DEVICE_LIMIT_REACHED` | 422 | — | Máximo de devices por owner. |
| `ACCOUNT_LOCKED` | 423 | — | Lockout temporal. |
| `RATE_LIMITED` | 429 | ✓ | Con `Retry-After` en REST. |
| `STALE_MESSAGE` | — | ✓ | `ts` fuera de ±5 min en `up/requests`. |
| `INTERNAL_ERROR` | 500 | ✓ | |
| `SERVICE_UNAVAILABLE` | 503 | — | PostgreSQL no disponible. |

## Catálogo — `error_code` de tareas

| Code | Quién lo asigna | Estado | `retryable` típico | Significado |
|---|---|---|---|---|
| `EXECUTION_ERROR` | Device | Failed | depende | Falla genérica al ejecutar. |
| `NOT_PERMITTED` | Device | Failed | false | Política local del device rechaza el tipo o payload. |
| `UNSUPPORTED_TASK_TYPE` | Device | Failed | false | El device no reconoce el tipo. |
| `INVALID_PAYLOAD` | Device | Failed | false | Payload no cumple el esquema del device. |
| `EXPIRED_ON_ARRIVAL` | Device | Failed | false | Llegó con `expiresAt` vencido. |
| `DELIVERY_FAILED` | Cloud | Failed | — | Se agotaron `max_attempts` sin ACK. |
| `EXECUTION_TIMEOUT` | Cloud | Failed | — | Superó `max_duration_seconds` (o 24 h). |
| `DEVICE_LOST` | Cloud | Failed | — | El device reconectó sin conocer la tarea entregada. |
| `RESULT_TOO_LARGE` | Cloud | Failed | — | `result` > 32 KiB. |
| `USER_REQUEST` | Cloud/Device | Cancelled | — | Cancelada por el owner o el device origen. |
| `APPROVAL_DENIED` | Device | Cancelled | — | Aprobación denegada. |
| `LOCAL_POLICY` | Device | Cancelled | — | El device la canceló por política local. |
| `DEVICE_REVOKED` | Cloud | Cancelled | — | Target revocado. |
| `NOT_DELIVERED` | Cloud | Expired | — | Venció `expires_at` sin entrega. |

`retryable` es informativo para el solicitante; **el Cloud nunca reintenta automáticamente una tarea `Failed`**. Reintentar es crear una tarea nueva (con otra idempotency key).
