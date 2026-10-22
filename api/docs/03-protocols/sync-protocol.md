# Protocolo de sincronización y memoria — `jarvis.sync/1`

Normativo. Decisiones en [ADR-013](../08-decisions/ADR-013-memory-module-in-cloud-core.md), [ADR-014](../08-decisions/ADR-014-edge-inference-and-device-embeddings.md) y [ADR-015](../08-decisions/ADR-015-sync-versions-and-conflicts.md). Los ejemplos completos y verificados por las pruebas de **ambos lados** (.NET y Rust) están en [`sync-vectors.json`](sync-vectors.json): si este documento y los vectores difieren, mandan los vectores y se corrige el documento.

Convenciones de la [REST API](rest-api.md): JSON en camelCase, errores en Problem Details, `X-Correlation-Id`, autenticación Bearer (token de user o de device) y versionado por ruta (`/api/v1`). El cuerpo del protocolo lleva además `"protocol": "jarvis.sync/1"`: un cambio incompatible crea `jarvis.sync/2` sin romper a los clientes anteriores.

## 1. Emparejar un device

El escritorio se registra con los endpoints existentes; no hay un `/devices/register` aparte (decisión de Phase 0: `POST /api/v1/devices`).

1. El owner inicia sesión desde el escritorio (`POST /api/v1/auth/login`). La contraseña no se guarda.
2. El escritorio se registra a sí mismo: `POST /api/v1/devices {"name":"…","type":"Windows"}`. El secreto se muestra **una vez** y va directo al Credential Manager de Windows.
3. El owner concede permisos de sincronización a ese device:

```http
PUT /api/v1/sync/devices/{deviceId}
Authorization: Bearer <token de user>

{"scopes": ["memories:read", "memories:write", "experiences:read", "experiences:write", "skills:read", "skills:write"]}
```

4. A partir de aquí el escritorio usa `POST /api/v1/auth/device-token` (deviceId + secreto) para obtener tokens de device de vida corta. Revocar, deshabilitar o rotar la credencial del device (endpoints existentes) corta la sincronización. Quitar los scopes (`PUT` con `[]`) la corta sin revocar el device.

| Scope | Permite |
|---|---|
| `memories:read` / `memories:write` | Recuerdos, relaciones y vectores |
| `experiences:read` / `experiences:write` | Experiencias |
| `skills:read` / `skills:write` | Habilidades (escritura: proponer versiones; verificar es solo del user) |

## 2. Entidades

| `entity` | `entityId` | Escritura | Fusión |
|---|---|---|---|
| `memory` | id del recuerdo | devices (`memories:write`) | Tres vías por campo; etiquetas como conjunto |
| `relation` | id de la relación | devices (`memories:write`) | No: conflicto si la versión base no coincide |
| `embedding` | id del recuerdo | devices (`memories:write`) | No aplica: uno por (recuerdo, modelo); se acepta solo si `contentHash` coincide con el contenido actual |
| `experience` | id de la experiencia | devices (`experiences:write`) | No |
| `skill` | id de la habilidad | devices (`skills:write`) y user | No |

Datos de cada entidad (`data`): ver `sync-vectors.json → entities`. Límites validados por el Cloud:

| Campo | Límite |
|---|---|
| `memory.content` | 1–8000 caracteres |
| `memory.summary` | ≤ 500 |
| `memory.tags` | ≤ 32 etiquetas de ≤ 64 caracteres |
| `confidence`, `relevance` | 0–1 |
| `memory.retention.kind` | `permanent` o `days`; `session` es local y se rechaza |
| `embedding.dims` | 1–4096 y igual al largo de `vector` |
| `embedding.contentHash` | SHA-256 en hex del contenido **exacto** en UTF-8, sin normalizar |
| Cambios por push | ≤ 200 y ≤ 1 MiB por petición |

El contenido con forma de secreto evidente (claves privadas PEM, `password: …`, tokens tipo `sk-…`, `ghp_…`, `AKIA…`) se rechaza con `CONTENT_REJECTED`. Es una segunda barrera: el escritorio ya los rechaza al guardar.

## 3. Push

```http
POST /api/v1/sync/push        (solo device)
Idempotencia: por changeId (no hace falta Idempotency-Key)
```

- Los cambios se aplican **en orden**, cada uno en su propia transacción. Un cambio rechazado no detiene a los siguientes.
- `baseVersion` es la versión del servidor sobre la que el device hizo el cambio (`null` = registro nuevo).

| `status` | Significado | El device debe… |
|---|---|---|
| `applied` | Aplicado tal cual. Devuelve `version` y `seq` | Guardar `version` como versión del servidor y quitarlo del outbox |
| `merged` | Aplicado tras fusionar con cambios concurrentes. Devuelve `data` final | Sobrescribir su copia local con `data` |
| `conflict` | No se aplicó: el mismo campo cambió en ambos lados, o borrado contra edición. Devuelve `conflictId`, `fields` y la versión actual del servidor | Conservar su copia, marcarla «en conflicto» y mostrarla al usuario |
| `rejected` | Inválido (`code`: `VALIDATION_FAILED`, `CONTENT_REJECTED`, `REFERENCE_NOT_FOUND`, `STALE_EMBEDDING`, `SYNC_SCOPE_MISSING`) | No reintentar; mostrar el motivo |

Un `changeId` repetido devuelve el **mismo resultado** que la primera vez con `"replayed": true`.

## 4. Pull

```http
POST /api/v1/sync/pull        (solo device)
{"protocol":"jarvis.sync/1","since":0,"limit":200,"embeddingModels":["qwen3-embedding:0.6b"]}
```

- Devuelve el **estado actual** de cada registro cambiado con `seq > since`, ordenado por `seq`. Si un registro cambió tres veces, aparece una vez con su último `seq`.
- Los borrados llegan como `op: "delete"` con `data: null` (tombstone).
- Los vectores solo se incluyen para los modelos pedidos en `embeddingModels`.
- Se repite con `since = nextSince` mientras `hasMore` sea `true`.
- Si `since` es anterior a la compactación de tombstones, la respuesta es `410 SYNC_CURSOR_EXPIRED` y el device hace un pull completo (`since: 0`).

El device ignora los cambios cuya `version` sea menor o igual a la que ya tiene (incluidos los suyos propios). Un registro local con cambios pendientes en el outbox **no se sobrescribe**: su push posterior se fusiona o produce un conflicto en el servidor.

## 5. Conflictos

```http
GET  /api/v1/sync/conflicts?status=open                 (user o device con memories:read)
POST /api/v1/sync/conflicts/{id}/resolve                (user o device con memories:write)
{"resolution": "keep_server" | "keep_device" | "custom", "data": { … solo con custom … }}
```

`kind`: `update_update`, `update_delete` (el device editó algo que ya estaba borrado), `delete_update` (el device borró algo que otro editó) y `create_create`.

Resolver produce una versión nueva (o un tombstone, si se elige el borrado), que se propaga por pull. Resolver dos veces devuelve `409 CONFLICT_ALREADY_RESOLVED`.

## 6. Memoria y recuperación

```http
GET  /api/v1/memories?kind=&q=&limit=&cursor=       (user o device con memories:read)
GET  /api/v1/memories/{id}                          (recuerdo + relaciones vigentes)
GET  /api/v1/memories/{id}/versions                 (historial)
POST /api/v1/memories/search
GET  /api/v1/memories/export                        (solo user; formato jarvis.memory-export/1)
```

`POST /api/v1/memories/search`:

| Campo | Uso |
|---|---|
| `query` | Texto (obligatorio): búsqueda de texto de PostgreSQL |
| `embedding` | Opcional `{model, vector}` calculado por el device con su modelo. Activa la búsqueda vectorial contra los vectores del mismo modelo y dimensión |
| `limit` | 1–50 (por defecto 8) |
| `minConfidence` | Por defecto 0,3. Los recuerdos `disputed` y los vencidos nunca se devuelven |
| `expandGraph` | Incluye vecinos verificados de los resultados (por defecto `true`) |

La respuesta incluye `method` (`vector:pgvector`, `vector:application` o `text`), los recuerdos con su puntuación y las relaciones entre ellos, para que el Neural Core muestre qué nodos y conexiones se usaron.

## 7. Experiencias y habilidades

```http
GET  /api/v1/experiences?signature=&limit=                    (experiences:read)
GET  /api/v1/experiences/similar?goal=texto&limit=            (soluciones anteriores para un objetivo)
GET  /api/v1/skills                                           (skills:read)
GET  /api/v1/skills/{id} · /api/v1/skills/{id}/versions
POST /api/v1/skills                                           (solo user)
PATCH /api/v1/skills/{id}  {"status":"verified"|"deprecated"} (solo user)
GET  /api/v1/skills/proposals                                 (solo user)
POST /api/v1/skills/proposals/{id}/accept | /reject           (solo user)
```

Una propuesta aceptada crea una habilidad en estado `draft`; marcarla `verified` es un acto explícito del user. Los devices reciben las habilidades por pull y **las guardan desactivadas y pendientes de revisión local**. El Cloud no las ejecuta.

## 8. Jobs

Ver [ADR-016](../08-decisions/ADR-016-worker-and-scheduled-jobs.md).

```http
GET  /api/v1/jobs · POST /api/v1/jobs · GET /api/v1/jobs/{id} · PATCH /api/v1/jobs/{id}
POST /api/v1/jobs/{id}/approve · POST /api/v1/jobs/{id}/run · GET /api/v1/jobs/{id}/runs
```

`GET /api/v1/tasks` sigue siendo el estado de las **tareas entre devices** (MQTT).

## 9. Salud y capacidades

```http
GET /api/v1/health     (anónimo; también lo usa el escritorio para medir latencia)
{"status":"ok","apiVersion":"v1","serverTime":"…","features":{"sync":"jarvis.sync/1","vectorSearch":"pgvector"}}
```

`/health/live` y `/health/ready` siguen como estaban (orquestación).

## 10. Transporte y resiliencia (obligaciones del cliente)

| Tema | Regla |
|---|---|
| TLS | Siempre HTTPS. HTTP plano solo hacia `localhost`/`127.0.0.1` (desarrollo) |
| Timeouts | Conexión 5 s, petición 20 s (push/pull), 3 s para la búsqueda durante una conversación |
| Reintentos | Errores de red, 429, 502, 503 y 504: backoff exponencial con jitter (1 s → 5 min). Nunca reintentar 400, 401, 403, 404, 409, 410 ni 422 sin cambiar algo |
| 401 | Pedir un token de device nuevo una vez. Si vuelve a fallar, el device está revocado o deshabilitado: parar y avisar |
| Desconexión | Seguir trabajando en local; el outbox crece y se vacía al volver |
| Rate limiting | 120 peticiones de sync por minuto por device (`429` con `Retry-After`) |
| Tamaño | Lotes de push ≤ 200 cambios y ≤ 1 MiB |

## 11. Eventos de auditoría

`SYNC_PUSH_COMPLETED` (contadores por estado), `SYNC_CONFLICT_DETECTED`, `SYNC_CONFLICT_RESOLVED`, `SYNC_SCOPES_CHANGED`, `SKILL_STATUS_CHANGED`, `JOB_RUN_STARTED`, `JOB_RUN_SUCCEEDED`, `JOB_RUN_FAILED`. Nunca incluyen contenido de recuerdos.
