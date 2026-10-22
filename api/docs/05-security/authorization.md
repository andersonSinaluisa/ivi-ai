# Authorization

## Modelo

Tres mecanismos, todos obligatorios:

1. **Owner scoping:** todo recurso pertenece a un `owner_id`; un principal solo ve recursos de su owner.
2. **Tipo de principal:** algunas operaciones solo las puede hacer un User (administración) y otras solo un Device (reportar estado de tareas).
3. **Grants de tareas:** un Device solo crea tareas `(target, taskType)` explícitamente concedidas.

No hay roles (admin/viewer) en MVP. Cuando existan varios users por owner (organización/familia) se agregará un modelo de membresía; ver [ADR-009](../08-decisions/ADR-009-multi-tenant-preparation.md).

## Implementación

| Capa | Mecanismo |
|---|---|
| Api | Esquema JWT único; políticas `UserOnly` (`typ=user`), `DeviceOnly` (`typ=device`), `UserOrDevice`. Aplicadas por grupo de endpoints. |
| Api | `ICurrentPrincipal` (id, tipo, ownerId) construido desde claims y verificado contra estado en BD (caché 30 s). |
| Application | Cada handler recibe `ICurrentPrincipal` y filtra por `OwnerId`. Además, `JarvisDbContext` aplica un **global query filter** por `owner_id` en `devices`, `tasks`, `device_task_grants`, `events` cuando hay principal (defensa en profundidad). Background services usan un contexto sin filtro explícitamente marcado (`SystemScope`). |
| Broker | ACL dynsec por `%u` ([MQTT Protocol §8](../03-protocols/mqtt-protocol.md#8-autenticación-autorización-y-acl)). |
| Ingress MQTT | Validación de `deviceId` del topic vs envelope y de pertenencia de la tarea. |

## Matriz REST

| Endpoint | User (owner) | Device | Anónimo |
|---|---|---|---|
| `POST /auth/login`, `/auth/refresh`, `/auth/device-token` | — | — | ✓ |
| `POST /auth/logout` | ✓ | ✗ | ✗ |
| `GET /me` | ✓ | ✓ | ✗ |
| `POST/GET/PATCH /devices…`, `/disable`, `/enable`, `/revoke`, `/credentials/rotate` | ✓ (propios) | ✗ 403 | ✗ 401 |
| `GET/POST/DELETE /devices/{id}/grants…` | ✓ (propios) | ✗ 403 | ✗ 401 |
| `POST /tasks` | ✓ target propio | ✓ con grant | ✗ 401 |
| `GET /tasks`, `GET /tasks/{id}` | ✓ todas del owner | ✓ solo donde es origen o target | ✗ 401 |
| `POST /tasks/{id}/cancel` | ✓ | ✓ solo si es origen | ✗ 401 |
| `GET /events` | ✓ | ✗ 403 | ✗ 401 |
| `/hubs/events` | ✓ | ✗ 403 | ✗ 401 |
| `/health/live` | ✓ | ✓ | ✓ |

Recurso inexistente o de otro owner → **404** en todos los casos.

## Matriz MQTT

| Acción | Device como origen | Device como target | Otro device |
|---|---|---|---|
| `task.create` hacia target T con tipo X | ✓ si existe grant `(self, T, X)` | — | — |
| `task.get` de la tarea | ✓ | ✓ | ✗ `NOT_FOUND` |
| `task.cancel_request` | ✓ | ✗ `FORBIDDEN` | ✗ `NOT_FOUND` |
| Mensajes `up/tasks` de la tarea | ✗ `SECURITY_VIOLATION` | ✓ | ✗ `SECURITY_VIOLATION` |
| Recibir `task.update` | ✓ | — | ✗ |

## Reglas de `POST /tasks` / `task.create`

Orden de evaluación (la primera que falla determina el error):

1. Autenticado y principal `Active`.
2. Validación de forma (`VALIDATION_FAILED`, `PAYLOAD_TOO_LARGE`, `IDEMPOTENCY_KEY_REQUIRED`).
3. Idempotencia: si la key existe → devolver la tarea existente o `IDEMPOTENCY_KEY_REUSED`.
4. Target existe y es del mismo owner (`NOT_FOUND`).
5. Si el principal es Device: grant `(self, target, type)` existe (`TASK_NOT_GRANTED`). Un device no puede crearse tareas a sí mismo.
6. Target `admin_state = Active` (`TARGET_DEVICE_UNAVAILABLE`).
7. `type ∈ target.supported_task_types` (`TASK_TYPE_NOT_SUPPORTED`).
8. Rate limit (`RATE_LIMITED`) — evaluado antes del paso 2 en la práctica, listado aquí por completitud.

Los rechazos en pasos 4-7 generan `TASK_REJECTED`.

## Grants

- Solo el owner los crea/borra.
- No existen comodines (`*`) ni grants por tipo de device: siempre `(source, target, taskType)` concretos.
- Revocar un device elimina sus grants.
- Ejemplo MVP: `(esp32-sim, windows-sim, OPEN_CHROME)`.

## Autorización en el device (fuera del Cloud)

El Cloud autoriza *quién puede pedir*. El device destino autoriza *qué se ejecuta*. El Windows Agent aplicará su Permission Engine sin importar lo que el Cloud haya aprobado. Ver [Windows Agent Contract](../07-integrations/windows-agent-contract.md).
