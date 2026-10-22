# ADR-013 — Memoria, conocimiento, experiencias y habilidades como módulos del Cloud Core

- **Status:** Accepted
- **Fecha:** 2026-10-22
- **Modifica:** [Roadmap](../00-overview/roadmap.md) Phase 8 («Memory como servicio separado del Cloud Core, con ADR propio de privacidad y almacenamiento»).

## Context

La app de escritorio (`assistant/`) ya tiene una memoria completa en SQLite (recuerdos tipados, grafo con procedencia, experiencias, habilidades versionadas, exportación cifrada). Vive en una sola PC: si se cambia de computadora, de modelo o de device, se pierde. El objetivo es que la memoria, las experiencias y las habilidades sobrevivan a esos cambios y que Jarvis siga funcionando sin Internet.

El roadmap original preveía un servicio de memoria separado. Un segundo servicio (otro proceso, otra base, otro despliegue, otra autenticación) duplica identidad, autorización y operación para un único owner.

## Decision

1. **Monolito modular.** Memoria, grafo de conocimiento, sincronización, experiencias, habilidades y jobs son módulos del mismo backend .NET (`Jarvis.Domain`/`Application`/`Infrastructure`/`Api`), con un **worker separado** (`Jarvis.Worker`, [ADR-016](ADR-016-worker-and-scheduled-jobs.md)) para el trabajo programado.
2. **Esquema PostgreSQL propio `memory`** (tablas `memories`, `memory_versions`, `memory_relations`, `memory_embeddings`, `experiences`, `skills`, `skill_versions`, `skill_proposals`, `sync_devices`, `sync_receipts`, `sync_conflicts`, `jobs`, `job_runs`). Mismo `owner_id` en todo ([ADR-009](ADR-009-multi-tenant-preparation.md)) y filtro global por owner.
3. **La fuente de verdad de cada device es su SQLite local**; el Cloud es la **réplica compartida** y el árbitro de versiones: asigna versión por registro y una secuencia de cambios por owner ([ADR-015](ADR-015-sync-versions-and-conflicts.md)).
4. **Independencia del modelo.** El contenido, las relaciones y las versiones no dependen de ningún modelo. Los vectores se guardan aparte, etiquetados con el modelo que los produjo ([ADR-014](ADR-014-edge-inference-and-device-embeddings.md)).
5. **Privacidad** (el ADR pendiente del roadmap):
   - Solo devices del owner con **permiso de sincronización explícito** (scopes por device: `memories:read`, `memories:write`, `skills:read`, `skills:write`, `experiences:read`, `experiences:write`) leen o escriben memoria.
   - Un device revocado o deshabilitado deja de sincronizar en ≤ 30 s (caché de `PrincipalStatusService`).
   - Al borrar un recuerdo se **purga su contenido y su historial** y queda un *tombstone* (id, versión, fecha) para propagar el borrado.
   - Los clientes no suben secretos (el escritorio ya los rechaza al guardar); el Cloud rechaza contenido con patrones de secreto evidentes como segunda barrera.
   - No viajan conversaciones completas, muestras de voz ni perfiles de voz.
   - El event log registra **metadatos** de sincronización (cuántos cambios, conflictos), nunca contenido.
6. **Las habilidades del registro nunca se ejecutan en el Cloud** ni se activan solas en un device: llegan desactivadas y el device exige revisión local.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Servicio de memoria separado (roadmap original) | Duplica autenticación, autorización, despliegue y backups para un solo owner. El aislamiento que buscaba se consigue con un esquema y grants propios |
| Base de datos de grafos (Neo4j) | El grafo personal (miles a decenas de miles de nodos) cabe en tablas con índices; recorridos acotados con `WITH RECURSIVE`. Otro motor más que operar sin necesidad |
| Memoria solo en el Cloud (thin client) | Rompe el funcionamiento sin Internet, que es requisito |
| Sincronizar el archivo SQLite completo | Sin resolución de conflictos por registro; transfiere todo en cada cambio |

## Consequences

- (+) Una sola identidad, un solo despliegue, un solo backup.
- (+) La memoria sobrevive al cambio de PC (se reconstruye por *pull*) y de modelo (el contenido no depende de él).
- (−) El backend crece; se mitiga con módulos separados por carpeta y pruebas por módulo.
- (−) El contenido de la memoria reside en el VPS: depende de la seguridad del VPS, TLS y backups cifrados. El cifrado de extremo a extremo del contenido queda como mejora futura (impediría la búsqueda de texto en el servidor).
