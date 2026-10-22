# Observability

Tres pilares con propósitos distintos:

| Pilar | Pregunta que responde | Persistencia | Audiencia |
|---|---|---|---|
| **Audit events** (`events`) | ¿Qué pasó con esta tarea/device y quién lo hizo? | PostgreSQL, 180 días | Owner, forense |
| **Logs** | ¿Por qué falló / qué hizo el sistema internamente? | Docker json-file, ~200 MB rotados | Operador |
| **Metrics** | ¿Está sano el sistema? ¿Tendencias? | En memoria / scrape futuro | Operador, alertas |

## Logging estructurado

- `Microsoft.Extensions.Logging` con formatter JSON de consola (sin Serilog: menos dependencias; se puede agregar si se necesita un sink).
- **Scopes obligatorios** (se agregan como propiedades a cada línea):

| Propiedad | Cuándo | Fuente |
|---|---|---|
| `CorrelationId` | Siempre | Header `X-Correlation-Id`, `X-Request-Id` de nginx, envelope MQTT, o generado |
| `TraceId` | Siempre | `Activity.Current` (W3C) |
| `OwnerId` | Si hay principal | Claims / tarea / device |
| `PrincipalType`, `PrincipalId` | Si hay principal | Claims |
| `DeviceId` | Flujos de device | Topic MQTT / tarea |
| `TaskId` | Flujos de tarea | |
| `MqttMessageId`, `MqttMessageType` | Ingress MQTT | Envelope |

- Nombres de mensajes estables tipo evento (`task_dispatched`, `mqtt_invalid_envelope`, `task_update_ignored`, `late_terminal_update`) con `EventId` numérico por categoría para poder filtrar.
- Niveles: `Information` para transiciones de negocio, `Warning` para rechazos de seguridad y anomalías, `Error` para fallos inesperados, `Debug` para contenido de protocolo (deshabilitado en producción).

### Datos prohibidos en logs

| Nunca | En su lugar |
|---|---|
| Contraseñas, secretos de device, pepper, claves | — |
| JWT, refresh tokens, header `Authorization` | `jti` del token |
| Query string de `/hubs/*` | Ruta sin query |
| `payload` / `result` de tareas | Tamaño en bytes y `type` |
| Email completo | `userId` (el email solo en `USER_LOGIN_FAILED` como hash) |
| IP en claro en eventos | HMAC diario (logs de nginx sí conservan IP: operativos, rotan en días) |

Implementación: los DTOs con datos sensibles no se pasan a `ILogger` (se loguean propiedades explícitas); `[LoggerMessage]` source-generated para mensajes frecuentes; test automatizado que falla si el secreto de un device aparece en la salida de logs durante el registro.

## Métricas

`System.Diagnostics.Metrics` con meter `Jarvis.Cloud`. Exposición: endpoint Prometheus `/metrics` en un **puerto interno separado** (`8081`, no publicado, no enrutado por nginx). Recolección/dashboards: post-MVP (el endpoint existe desde el MVP para depuración con `curl` desde el host).

| Métrica | Tipo | Tags |
|---|---|---|
| `jarvis_tasks_created_total` | counter | `channel` (rest/mqtt), `requested_by_type` |
| `jarvis_tasks_terminal_total` | counter | `status`, `error_code` |
| `jarvis_task_dispatch_attempts_total` | counter | `attempt` |
| `jarvis_task_delivery_latency_seconds` | histogram | — (created → delivered) |
| `jarvis_task_execution_duration_seconds` | histogram | `status` (started → completed) |
| `jarvis_tasks_pending` | gauge | — |
| `jarvis_devices_by_presence` | gauge | `presence_state` |
| `jarvis_mqtt_messages_received_total` | counter | `message_type`, `result` (ok/invalid/rejected) |
| `jarvis_mqtt_messages_published_total` | counter | `message_type` |
| `jarvis_mqtt_connected` | gauge | 0/1 del cloud client |
| `jarvis_security_violations_total` | counter | `rule` |
| `jarvis_auth_failures_total` | counter | `kind` (user/device) |
| ASP.NET Core / Kestrel / runtime | built-in | — |

Tags con cardinalidad acotada: nunca `deviceId`, `taskId` ni `ownerId` como tag.

## Health checks

| Endpoint | Checks | Resultado |
|---|---|---|
| `/health/live` | Ninguno (el proceso responde) | 200 siempre que el host HTTP funcione |
| `/health/ready` | ver tabla | 200 `Healthy`/`Degraded`, 503 `Unhealthy` |

Readiness:

| Dependencia | Check | Si falla | Motivo |
|---|---|---|---|
| PostgreSQL | `SELECT 1` con timeout 2 s | **Unhealthy** (503) | Sin fuente de verdad la API no puede operar. |
| Migraciones | La última migración conocida por el binario está aplicada (verificado al arrancar y cacheado) | **Unhealthy** | Ejecutar con esquema viejo corrompe datos. |
| Cloud client MQTT | Conectado y suscrito | **Degraded** (200) | La REST API sigue siendo útil (consultas, crear tareas que quedarán `Pending`). |
| Reconciliación dynsec inicial | Completada al menos una vez | **Degraded** | |
| Background services | Último ciclo de cada sweeper/dispatcher hace < 3× su intervalo | **Degraded** | Detecta un hosted service colgado. |

Nginx **no** usa readiness para enrutar (una sola instancia). Readiness sirve para el healthcheck de Docker, para el paso de verificación de despliegue y para monitoreo.

Formato de respuesta: JSON con estado por check (sin mensajes de excepción, sin cadenas de conexión).

## Audit events

Catálogo en [Event Model](../02-domain/event-model.md). Reglas de observabilidad:
- Toda transición de tarea y toda operación administrativa genera un evento en la misma transacción.
- `correlation_id` en todas las filas; permite unir events ↔ logs ↔ mensajes MQTT.
- Consulta de reconstrucción de una tarea: `GET /api/v1/events?taskId={id}` o por `correlationId`.

## Alertas (MVP mínimo)

Sin stack de monitoreo en MVP. Monitoreo externo gratuito/sencillo (p. ej. un uptime checker contra `/health/live` y el puerto 8883) + script en cron del host que revise:

| Condición | Umbral |
|---|---|
| `/health/live` no responde | 2 chequeos consecutivos |
| Disco usado | > 80 % |
| Certificado expira | < 14 días |
| Backup diario no generado | > 26 h desde el último |

Canal de alerta: email del owner (decisión pendiente, ver [docs/README.md](../README.md#decisiones-pendientes-antes-de-implementar)).
