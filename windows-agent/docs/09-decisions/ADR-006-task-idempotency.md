# ADR-006: Idempotencia por `taskId`

## Status

Accepted, 2026-09-19.

## Context

MQTT QoS 1 y los reintentos del Cloud entregan la misma tarea varias veces. Re-ejecutar acciones con efectos (abrir, cerrar, ejecutar comandos) es inaceptable. Exactly-once no existe en sistemas distribuidos con fallos.

## Decision

- **At-least-once de entrega + como mucho una ejecución por `taskId`** (efecto "exactly-once" local).
- Dedup por `taskId` en SQLite **antes** de expiración y validación; una tarea conocida recibe su último estado persistido, nunca una segunda ejecución.
- `executionId` (UUIDv7) por ejecución, 0..1 por tarea, garantizado con `UNIQUE(task_id)`.
- `Running` se persiste antes de ejecutar; tras un crash se reporta `EXECUTION_INTERRUPTED` y no se re-ejecuta.
- Retención 8 días tras *settled*; guardia `db_created_at` contra reinicios de la BD.
- Sin `IdempotencyKey` propia: el Cloud ya convierte la `Idempotency-Key` REST en un `taskId` único.

Detalle en [Idempotency](../06-state/idempotency.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Dedup por `envelope.id` | Cada reintento del Cloud puede traer otro `id` |
| Re-ejecutar tras crash si la tool es "idempotente" | Idempotencia del efecto no es fiable en Windows (una app abierta dos veces son dos ventanas) y oculta fallos |
| Dedup solo en memoria | Pierde la memoria al reiniciar, justo cuando más redeliveries hay |

## Consequences

- ✅ Ningún redelivery produce una segunda acción.
- ⚠️ Una tarea interrumpida por un crash no se completa sola: el solicitante debe reintentarla (nuevo `taskId`).
- ⚠️ Tras una recreación de la BD se rechazan tareas antiguas legítimas.
