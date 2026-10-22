# ADR-007 — JSONB usage

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Algunos datos tienen estructura variable por tipo de tarea o de device; otros son fijos y se consultan/filtran. Abusar de JSONB degrada integridad y consultas; evitarlo por completo obliga a tablas EAV o a migraciones por cada tipo de tarea nuevo.

## Decision

JSONB **solo** donde la estructura la define un actor externo al Cloud o varía por tipo:

| Columna | Motivo |
|---|---|
| `tasks.payload` | Definido por el tipo de tarea; opaco para el Cloud. |
| `tasks.result` | Definido por el device ejecutor; opaco. |
| `devices.reported_info` | Metadatos reportados que varían por tipo de device (firmware, OS, hardware). |
| `events.data` | Detalle específico de cada tipo de evento, pequeño. |

Todo lo que el Cloud **consulta, filtra, indexa o valida** es columna tipada: estados, fechas, ids, `type`, `correlation_id`, `idempotency_key`, `error_code`, progreso.

Listas simples de etiquetas (`capabilities`, `supported_task_types`) usan `text[]`, no JSONB ([Device Model](../02-domain/device-model.md#capabilities-decisión-de-modelado)).

Reglas: `CHECK (jsonb_typeof(x) = 'object')`, límites de tamaño en la app y en BD, sin índices GIN en MVP (no se consulta dentro del JSON).

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Tabla por tipo de tarea | Acopla el Cloud a la semántica de tareas (contradice ADR-010) y exige migraciones por tipo. |
| `text` con JSON serializado | Sin validación ni capacidad futura de consulta. |
| Todo el agregado en JSONB | Pierde constraints, FK e índices eficientes. |

## Consequences

- (+) Nuevos tipos de tarea no requieren cambios en el Cloud.
- (+) Integridad fuerte en todo lo que el Cloud razona.
- (−) El Cloud no puede validar el contenido de `payload` (por diseño; la validación es del device).
