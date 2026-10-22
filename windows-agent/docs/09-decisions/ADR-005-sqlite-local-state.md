# ADR-005: SQLite para el estado local

## Status

Accepted, 2026-09-19.

## Context

El contrato del Cloud exige: recordar `taskId` ≥ 8 días (W7), no re-ejecutar, reenviar estados hasta `task.update.ack` (W6) y reportar ejecuciones interrumpidas (W9). Todo debe sobrevivir crashes, reinicios y cortes de energía, con atomicidad entre "estado de la tarea" y "mensaje pendiente".

## Decision

SQLite embebido (`rusqlite` con `bundled`) en `%LOCALAPPDATA%\Jarvis\Agent\state\agent.db`, WAL, `synchronous=FULL`, escritor único, migraciones embebidas. Tablas `tasks`, `executions` (UNIQUE `task_id`), `approvals`, `outbox`, `meta`. Configuración y política quedan en TOML; credenciales en Credential Manager. Detalle en [Local State](../06-state/local-state.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Archivos JSON | Sin transacciones; corrupción parcial ante cortes |
| sled / redb / RocksDB | KV sin SQL ni herramientas de inspección; más riesgo de madurez |
| Solo memoria | Incumple W6/W7/W9 |
| Registro de Windows | No es una base de datos transaccional para este volumen |

## Consequences

- ✅ Atomicidad estado + outbox; inspección con herramientas estándar.
- ⚠️ Corrupción posible (rara) → plan de [Crash Recovery](../06-state/crash-recovery.md) y guardia `STATE_RESET`.
- ⚠️ `synchronous=FULL` cuesta latencia de fsync (ms), irrelevante para el volumen de tareas.
