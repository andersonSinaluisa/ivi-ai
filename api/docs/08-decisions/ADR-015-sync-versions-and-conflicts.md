# ADR-015 — Sincronización: outbox local, versiones del servidor, fusión a tres vías y tombstones

- **Status:** Accepted
- **Fecha:** 2026-10-22

## Context

Cada device escribe primero en su SQLite (funciona sin Internet) y sincroniza después. Dos devices pueden modificar el mismo recuerdo sin conexión. Los relojes de los devices no son fiables, y el encargo exige no sobrescribir información en silencio.

## Decision

1. **Outbox local.** Cada cambio local (alta, edición, borrado) se registra en la tabla `sync_outbox` del device **en la misma transacción** que el cambio. Cada entrada lleva:
   - un `changeId` (UUID) que la identifica para siempre y hace idempotente su envío;
   - la `baseVersion`: la versión del servidor sobre la que se hizo el cambio (`null` si el registro es nuevo).
2. **Versiones asignadas por el servidor.**
   - Cada registro tiene una `version` entera que solo incrementa el Cloud.
   - Cada cambio aceptado recibe un `seq` de una secuencia de PostgreSQL. La asignación se serializa por owner con un *advisory lock* de transacción, así el orden de `seq` coincide con el de commit.
   - Los devices leen cambios con `seq > cursor`.
   - **Los timestamps del cliente se guardan solo como dato informativo**: nunca deciden un conflicto.
3. **Idempotencia.** El resultado de cada `changeId` se guarda en `sync_receipts`. Un reenvío (respuesta perdida, reintento) devuelve el mismo resultado sin aplicar nada dos veces.
4. **Detección de conflictos.**
   - Si `baseVersion` coincide con la versión actual, el cambio se aplica.
   - Si no coincide:
     - **Recuerdos:** fusión a tres vías por campo, con la versión base tomada del historial (`memory_versions`).
       - Si cada lado cambió campos distintos, se combinan y el resultado es `merged`.
       - Las etiquetas se fusionan como conjuntos: se suman las añadidas y se quitan las eliminadas por cualquiera de los lados.
       - Si ambos cambiaron el mismo campo con valores distintos, el resultado es `conflict`.
     - **Relaciones, experiencias y habilidades:** sin fusión; cualquier diferencia de versión con contenido distinto es `conflict`.
     - **Borrado contra edición** (en cualquier orden) es siempre `conflict`.
5. **Un conflicto nunca sobrescribe.**
   - El Cloud conserva su versión y guarda la propuesta del device en `sync_conflicts`: datos completos, campos en disputa y versión base.
   - El device conserva su versión local y marca el registro como «en conflicto».
   - Cualquier device del owner o la UI lo resuelve con `POST /api/v1/sync/conflicts/{id}/resolve` (quedarse con la del servidor, con la del device o con un contenido nuevo). Esa resolución es una versión nueva que se propaga como cualquier cambio.
6. **Tombstones.** Borrar un registro incrementa su versión y su `seq`, guarda `deletedAt` y purga el contenido y el historial. El tombstone se propaga por *pull* para que todos los devices borren. Una edición posterior sobre un tombstone es `conflict` (la propuesta queda guardada por si el usuario quiere restaurarla). Los tombstones se conservan 180 días (configurable) y luego se compactan; un device que lleve más tiempo sin sincronizar recibe `SYNC_CURSOR_EXPIRED` y hace un *pull* completo.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| *Last-writer-wins* por timestamp | Pierde datos en silencio y depende de relojes de cliente |
| CRDT completos | Complejidad alta para texto libre; la fusión por campo con conflictos explícitos cubre el caso personal |
| Vector clocks por device | El servidor es siempre el punto de encuentro; una versión por registro es suficiente y más simple de razonar |
| Borrado físico inmediato | Los devices sin conexión nunca se enterarían del borrado |

## Consequences

- (+) Ningún cambio se pierde sin que el usuario lo vea.
- (+) Reintentos seguros ante cortes de red.
- (−) El usuario tiene que resolver los conflictos reales (mismo campo, valores distintos). Se espera que sean raros con un solo usuario.
- (−) `memory_versions` crece con cada edición. Se compacta con la retención (los recuerdos borrados no dejan historial).
