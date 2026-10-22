# EPIC-10 — Resilience

Fase **P6**. Objetivo: comportamiento definido y probado ante todos los fallos de [Failure Scenarios](../../11-testing/failure-scenarios.md). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-043 — Corrupción de SQLite y guardia de reinicio

**Goal**: sobrevivir a una BD corrupta sin re-ejecutar tareas antiguas.

**Description**: [Corrupción de SQLite](../../06-state/crash-recovery.md#corrupción-de-sqlite) y [Reinicio de la base de datos](../../06-state/idempotency.md#reinicio-de-la-base-de-datos).

**Acceptance Criteria**
1. `quick_check` fallido o `SQLITE_CORRUPT`/`SQLITE_NOTADB` → BD renombrada a `agent.db.corrupt-<timestamp>`, nueva BD, `db_created_at` nuevo, condición `STATE_RESET`.
2. Durante la operación: la ingesta se detiene, se completa el procedimiento y se reanuda sin reiniciar el proceso.
3. Tareas con `createdAt < db_created_at` → `task.rejected EXPIRED_ON_ARRIVAL` (motivo `STATE_RESET`).
4. `STATE_RESET` visible hasta que el owner lo reconozca (`status --ack-reset`).
5. Backup `agent.db.bak` antes de cada migración de esquema.

**Dependencies**: HU-WIN-015, HU-WIN-038.

**Technical Considerations**: cerrar todas las conexiones antes de renombrar (Windows bloquea archivos abiertos).

**Tests**: corromper bytes de la cabecera y de páginas intermedias en un archivo de prueba; reenviar un dispatch antiguo tras el reset.

**Definition of Done**: DoD común + escenario "SQLite corrupted" de [Failure Scenarios](../../11-testing/failure-scenarios.md) verde.

---

## HU-WIN-044 — Suite de escenarios de fallo (chaos)

**Goal**: comprobar automáticamente el comportamiento ante cada fallo documentado.

**Description**: automatizar los escenarios F-01…F-10 de [Failure Scenarios](../../11-testing/failure-scenarios.md).

**Acceptance Criteria**
1. Cloud offline, MQTT offline, pérdida de Internet (proxy TCP que corta), reinicio de Windows (simulado con kill + reinicio del proceso), crash del agente, corte de energía durante una tarea (kill en `Running`), resultado no enviado, mensaje MQTT duplicado, reloj incorrecto, SQLite corrupto.
2. Cada escenario tiene aserciones sobre el estado final en el Cloud y en la BD local.
3. La suite corre en CI (los que no necesitan escritorio) y en el runner interactivo (los que usan tools reales).
4. Resultados enlazados desde el checklist del MVP.

**Dependencies**: HU-WIN-042, HU-WIN-043.

**Technical Considerations**: `toxiproxy` (o un proxy TCP propio en Rust) entre el agente y el broker; reloj del agente desplazable por configuración de test.

**Tests**: la propia suite.

**Definition of Done**: DoD común.

---

## HU-WIN-045 — Protección de recursos y tareas atascadas

**Goal**: que ni un flood de mensajes ni una tool colgada degraden la PC o el agente.

**Description**: límites de [Concurrency Model](../../01-architecture/concurrency-model.md#límites-globales) y condición `TOOL_STUCK` de [Health](../../07-observability/health.md#condiciones).

**Acceptance Criteria**
1. Cola llena → la tarea espera en `Accepted`; si el tiempo en cola supera el timeout efectivo → `Failed TOOL_TIMEOUT`.
2. Tool abandonada tras `maxMs` + gracia → `TOOL_STUCK`; si hay 2 tools atascadas, el agente deja de aceptar nuevas tareas del mismo tipo hasta reiniciar.
3. Límite de memoria del proceso vigilado (Job Object propio con `JOB_OBJECT_LIMIT_PROCESS_MEMORY` de 512 MB configurables); al acercarse, `WARN`.
4. Consumo en reposo: < 1 % CPU y < 50 MB de working set medido durante 1 h.
5. Logs agregados ante floods (HU-WIN-003) y descarte de mensajes excedentes (HU-WIN-010).

**Dependencies**: HU-WIN-023, HU-WIN-039.

**Technical Considerations**: asignar el propio proceso a un Job Object es compatible con la Scheduled Task (Windows 8+ permite jobs anidados).

**Tests**: flood de 10 000 mensajes; `noop` que ignora la cancelación; medición de consumo en el runner interactivo.

**Definition of Done**: DoD común.
