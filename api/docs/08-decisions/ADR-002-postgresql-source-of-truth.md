# ADR-002 — PostgreSQL as source of truth

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Las tareas no deben perderse si un device está offline, MQTT se desconecta, el VPS o el broker reinician. Hay varios lugares donde podría vivir estado: PostgreSQL, colas del broker (sesiones persistentes, retained), memoria de la API, Redis, SignalR.

## Decision

**PostgreSQL 17 es la única fuente de verdad** para users, devices, credenciales, grants, tareas (incluido su estado de entrega) y eventos.

- El broker es transporte: devices con clean start; ningún mensaje retained; el Cloud redespacha desde PostgreSQL.
- La memoria de la API solo contiene cachés y señales reconstruibles.
- La tabla `tasks` actúa como outbox de dispatch; `events` como fuente de catch-up realtime.
- Todo cambio de estado y su evento se escriben en la misma transacción.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Colas persistentes del broker (clean session false para devices) | El broker no sabe de expiración de negocio ni de cancelaciones; entregaría comandos viejos al reconectar; estado duplicado difícil de auditar. |
| Broker con persistencia fuerte (EMQX, RabbitMQ) como fuente | Más recursos y operación; la auditoría seguiría necesitando BD. |
| Event Sourcing | Complejidad (proyecciones, versionado de eventos) sin necesidad: se requiere auditoría, no reconstrucción temporal. |
| SQLite | Suficiente en volumen, pero peor para concurrencia de background services, sin `SKIP LOCKED`, menos herramientas de backup online. |

## Consequences

- (+) Recuperación simple y determinista tras cualquier reinicio.
- (+) Una sola base para backup y auditoría.
- (−) Heartbeats y presencia escriben en PostgreSQL (decenas de escrituras/min: irrelevante; revisar a >1000 devices, ver ADR-008).
- (−) Latencia de dispatch depende de una transacción (ms), aceptable.
- Excepción documentada: las contraseñas de dynsec viven en el volumen de Mosquitto ([Persistence Strategy](../04-data/persistence-strategy.md)).
