# EPIC-07 — Events & Audit

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [Event Model](../../02-domain/event-model.md).

---

## HU-038 — Infraestructura del event log

- **Objetivo:** que cualquier caso de uso registre eventos en la misma transacción sin código repetido.
- **Descripción:** tabla `events` ([Database Design](../../04-data/database-design.md#events)); las entidades acumulan eventos de dominio; un interceptor de `SaveChanges` los convierte en filas `events` con `correlation_id`, actor y owner del contexto actual; toma `pg_advisory_xact_lock` por owner antes de insertar; después del commit, publica los eventos a un dispatcher in-process (usado por realtime y notificaciones MQTT).
- **Criterios de aceptación:**
  1. Un rollback no deja eventos; un commit no puede quedar sin sus eventos.
  2. `data` > 4 KiB → error en desarrollo (bug), nunca truncado silencioso.
  3. El rol `jarvis_app` no puede `UPDATE`/`DELETE` en `events` (test).
  4. Para un owner, el orden de `seq` coincide con el orden de commit (test concurrente).
  5. El dispatcher post-commit solo recibe eventos confirmados.
- **Dependencias:** HU-003.
- **Consideraciones técnicas:** `ICorrelationContext` y `ICurrentPrincipal`/`SystemActor` como fuentes del actor.
- **Pruebas esperadas:** integración con PostgreSQL.
- **Definition of Done:** DoD común.

---

## HU-039 — Consultar eventos por REST

- **Objetivo:** `GET /api/v1/events` según [REST API](../../03-protocols/rest-api.md#get-apiv1events).
- **Descripción:** filtros `after`, `type`, `deviceId`, `taskId`, `correlationId`, `from`, `to`, `limit`; orden ascendente con `after`, descendente sin él; `lastSeq`, `hasMore`.
- **Criterios de aceptación:**
  1. Solo eventos del owner; eventos con `owner_id` nulo nunca se devuelven.
  2. `?taskId=` reconstruye la historia completa de una tarea del caso MVP (creado → despachado → entregado → iniciado → completado).
  3. Principal Device → 403.
- **Dependencias:** HU-038, HU-043.
- **Consideraciones técnicas:** índices por owner/task/device/correlation.
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-040 — Retención de eventos y tokens

- **Objetivo:** controlar crecimiento de la BD.
- **Descripción:** job diario con conexión del rol `jarvis_maintenance`: borra `events` con `recorded_at` > 180 días y `refresh_tokens` expirados/revocados hace > 30 días, en lotes de 5 000.
- **Criterios de aceptación:**
  1. Períodos configurables.
  2. Ejecución no bloquea la API (lotes pequeños, pausa entre lotes).
  3. Log con cantidades borradas.
- **Dependencias:** HU-038.
- **Consideraciones técnicas:** conexión separada con otro connection string.
- **Pruebas esperadas:** integración con reloj falso.
- **Definition of Done:** DoD común.
