# Architecture Decision Records

| ADR | Título | Status |
|---|---|---|
| [ADR-001](ADR-001-backend-technology.md) | Backend technology (.NET 10 LTS, Minimal APIs, EF Core) | Accepted |
| [ADR-002](ADR-002-postgresql-source-of-truth.md) | PostgreSQL as source of truth | Accepted |
| [ADR-003](ADR-003-mqtt-device-communication.md) | MQTT (Mosquitto + Dynamic Security) for device communication | Accepted |
| [ADR-004](ADR-004-rest-signalr-responsibilities.md) | REST + SignalR responsibilities | Accepted |
| [ADR-005](ADR-005-device-authentication.md) | Device authentication (secreto por device) | Accepted |
| [ADR-006](ADR-006-task-delivery-semantics.md) | Task delivery semantics (at-least-once + idempotencia) | Accepted |
| [ADR-007](ADR-007-jsonb-usage.md) | JSONB usage | Accepted |
| [ADR-008](ADR-008-redis-decision.md) | Redis decision (no en MVP) | Accepted |
| [ADR-009](ADR-009-multi-tenant-preparation.md) | Multi-tenant preparation (`owner_id` en todo) | Accepted |
| [ADR-010](ADR-010-llm-independence.md) | LLM independence | Accepted |
| [ADR-011](ADR-011-native-postgres-nginx.md) | PostgreSQL y nginx nativos en producción; Docker solo para API y Mosquitto | Accepted |
| [ADR-012](ADR-012-end-to-end-task-signing.md) | Firma end-to-end de tareas y aprobaciones: el Cloud transporta, no firma ni verifica | Accepted |
| [ADR-013](ADR-013-memory-module-in-cloud-core.md) | Memoria, grafo, sync, experiencias, habilidades y jobs como módulos del Cloud Core (esquema `memory`) | Accepted |
| [ADR-014](ADR-014-edge-inference-and-device-embeddings.md) | Vectores etiquetados por modelo y búsqueda con el vector de la consulta | Accepted · puntos 1 y 5 sustituidos por ADR-017 |
| [ADR-015](ADR-015-sync-versions-and-conflicts.md) | Versiones del servidor, fusión a tres bandas y conflictos visibles | Accepted |
| [ADR-016](ADR-016-worker-and-scheduled-jobs.md) | Worker separado y jobs de un catálogo cerrado | Accepted |
| [ADR-017](ADR-017-server-model-services.md) | Modelos, transcripción y voz en el servidor con la credencial del device (nginx `auth_request`) | Accepted |

## Proceso

- Un ADR nuevo por cada decisión que cambie contratos, tecnología o semántica.
- Un ADR aceptado no se edita en su decisión: se crea uno nuevo con `Status: Supersedes ADR-00X` y el anterior pasa a `Superseded by ADR-00Y`.
- Formato: Status · Context · Decision · Alternatives · Consequences.
