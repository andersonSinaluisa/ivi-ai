# ADR-016 — Worker separado y jobs programados con handlers registrados

- **Status:** Accepted
- **Fecha:** 2026-10-22

## Context

El encargo pide tareas autónomas en el VPS: consolidar memoria, comprobar APIs, verificar backups, preparar reportes, sincronizaciones. Deben ser persistentes (sobrevivir a reinicios), con reintentos, timeouts, concurrencia controlada y auditoría, y **sin ejecutar código generado por un modelo**. El término «task» ya designa en el Cloud las tareas entre devices (MQTT, máquina de estados T1–T17).

## Decision

1. **Nombre distinto: *jobs*.** `jobs` (definición: tipo, programación, parámetros, modo, aprobación) y `job_runs` (cada ejecución: intento, estado, inicio, fin, resultado, evidencia, error). REST en `/api/v1/jobs`.
2. **Proceso separado `Jarvis.Worker`** (otro contenedor), con el rol de base de datos `jarvis_worker`. La API solo crea, lista, aprueba y consulta jobs. El worker:
   - materializa las ejecuciones vencidas;
   - las reclama con `FOR UPDATE SKIP LOCKED` y una **lease** con vencimiento;
   - las ejecuta con un timeout por job.

   Si el worker muere, la lease vence y otra instancia, o la misma al reiniciar, reintenta con `attempt + 1` hasta `maxAttempts`.
3. **Solo handlers registrados en código.** El tipo de job elige una clase del worker; los parámetros se validan con un esquema por tipo. No existe un tipo «ejecutar script», «shell» ni «evaluar código». Handlers iniciales:

   | Tipo | Modo | Requiere aprobación |
   |---|---|---|
   | `report.summary` | solo lectura | No |
   | `backup.verify` | solo lectura | No |
   | `http.check` | solo lectura (URL de una allowlist de configuración) | No |
   | `memory.consolidate` (duplicados → relaciones sugeridas sin borrar nada; experiencias → propuestas de habilidad) | modifica | No (solo añade sugerencias sin verificar) |
   | `sync.compact` (recibos de idempotencia y tombstones vencidos) | modifica | No |
   | `memory.purge_expired` (aplica la retención: tombstones de recuerdos vencidos) | modifica | **Sí** |

4. **Aprobación.** Un job sensible queda en `approvalState = Pending` y no se ejecuta hasta que el owner lo aprueba (`POST /api/v1/jobs/{id}/approve`). Un cambio de parámetros vuelve a pedir aprobación.
5. **Concurrencia.** Una sola ejecución activa por job (índice único parcial). Además, los handlers declaran una clave de concurrencia (por ejemplo, un `memory.consolidate` por owner).
6. **Auditoría.** Cada ejecución queda en `job_runs` y emite eventos `JOB_RUN_STARTED`, `JOB_RUN_SUCCEEDED`, `JOB_RUN_FAILED` en el event log (metadatos, sin contenido).

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Hosted services dentro de la API | Un trabajo pesado (consolidar) competiría con las peticiones; un fallo del job reinicia la API |
| Quartz.NET / Hangfire | Añaden su propio esquema y modelo; el patrón `SKIP LOCKED` ya se usa para el despacho de tareas |
| Que el modelo genere scripts para el VPS | Prohibido por el encargo y por ADR-010 |

## Consequences

- (+) Reinicios y caídas no pierden trabajos.
- (+) Superficie de ejecución cerrada y auditable.
- (−) Cada tipo de job nuevo requiere código y despliegue (deliberado).
