# EPIC-02 — Cloud Connectivity

Fase **P2**. Objetivo: el device aparece `Online` en Jarvis Cloud con presencia correcta y se recupera solo de cualquier corte. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-006 — Crate `jarvis-protocol` y contract tests

**Goal**: representar el protocolo MQTT v1 del Cloud sin ambigüedad y detectar desviaciones automáticamente.

**Description**: tipos Rust para el envelope, todos los mensajes `up/*` y `down/*` usados por el agente y los topics, según [MQTT Protocol del Cloud](../../../../api/docs/03-protocols/mqtt-protocol.md) y [Task Protocol](../../02-cloud-integration/task-protocol.md).

**Acceptance Criteria**
1. Envelope `{ v, id, type, ts, deviceId, correlationId?, data }` con `v == 1`.
2. Mensajes: `presence.hello|heartbeat|goodbye|offline`, `presence.hello.ack`, `task.dispatch|cancel|update.ack`, `task.ack|started|progress|approval_required|completed|failed|rejected|cancelled`.
3. Topics generados por funciones (`topics::up_tasks(device_id)`), nunca strings sueltos.
4. Deserialización tolerante a campos nuevos del envelope y estricta en `data` de mensajes entrantes críticos.
5. Fixtures JSON copiados de los ejemplos de la documentación del Cloud y de los tests del Cloud (`api/tests`) en `jarvis-protocol/tests/fixtures/`; round-trip de todos.

**Dependencies**: HU-WIN-001.

**Technical Considerations**: `#[serde(tag = "type")]` para el envelope; `time` o `chrono` con RFC 3339 y milisegundos; UUID como `uuid::Uuid`.

**Tests**: round-trip de fixtures; test que falla si un ejemplo de la documentación del Cloud cambia (hash de fixtures revisado).

**Definition of Done**: DoD común + cualquier diferencia encontrada con el Cloud queda registrada en [Cloud Compatibility](../../00-overview/cloud-compatibility.md).

---

## HU-WIN-007 — Conexión MQTT/TLS

**Goal**: conexión segura y autenticada con el broker.

**Description**: Cloud Link sobre `rumqttc` v5 según [MQTT Client](../../02-cloud-integration/mqtt-client.md).

**Acceptance Criteria**
1. TLS con `rustls` + `rustls-platform-verifier` (almacén de Windows); certificado inválido o hostname distinto → sin conexión y log `ERROR`.
2. `client_id` y `username` = `deviceId`; password desde el proveedor de credenciales (fake en esta HU, real en HU-WIN-014).
3. LWT `presence.offline { sessionId }` en `up/presence`, QoS 1.
4. Suscripción `jarvis/v1/devices/{deviceId}/down/#` QoS 1.
5. Event loop supervisado: un error del loop no termina el proceso.
6. `clean start = true` y keepalive de 60 s, según la tabla de parámetros de MQTT Client.

**Dependencies**: HU-WIN-006, HU-WIN-004.

**Technical Considerations**: el event loop de `rumqttc` debe sondearse continuamente en su propia tarea; la publicación se hace por canal hacia esa tarea.

**Tests**: integración con Mosquitto real en contenedor (Linux CI) con TLS de prueba y CA propia (inyectada solo en tests); test de certificado inválido.

**Definition of Done**: DoD común.

---

## HU-WIN-008 — Presencia: hello, heartbeat, goodbye

**Goal**: que el Cloud conozca en todo momento si el agente está disponible y qué soporta.

**Description**: implementar [Presence](../../02-cloud-integration/presence.md).

**Acceptance Criteria**
1. Tras CONNACK: `presence.hello` con `sessionId` nuevo, `protocolVersion "1.1"`, `softwareVersion`, `platform "windows"`, `capabilities`, `supportedTaskTypes` (del registro; vacío hasta EPIC-05), `openTaskIds`, `busy`, `sessionState`.
2. `presence.hello.ack` procesado: `heartbeatIntervalSeconds`, `serverTime` → offset de reloj (compensando RTT/2); condición `CLOCK_SKEW` si el desfase supera 5 min.
3. Heartbeat periódico y heartbeat inmediato ante cambio de `busy`/`sessionState` (máx. 1 cada 5 s).
4. `presence.goodbye` en apagado ordenado con `reason`.
5. Nunca se publican hostname, usuario, IP ni versión exacta de Windows (test que inspecciona el JSON).

**Dependencies**: HU-WIN-007.

**Technical Considerations**: reloj monotónico para intervalos; `nowCorregido` expuesto como servicio `Clock` inyectable.

**Tests**: integración con el Jarvis Cloud real (Testcontainers o `docker compose` del repo `api/`): el device aparece `Online`; matar el proceso → `Offline` por LWT.

**Definition of Done**: DoD común.

---

## HU-WIN-009 — Reconexión y backoff

**Goal**: recuperarse sin intervención de cualquier pérdida de conexión.

**Description**: implementar [Reconnection](../../02-cloud-integration/reconnection.md).

**Acceptance Criteria**
1. Backoff exponencial 1 s → 60 s con jitter ±20 %, reiniciado tras un `hello.ack`.
2. CONNACK de autenticación rechazada → condición `AUTH_REJECTED`, backoff fijo de 5 min, `WARN` una vez por hora.
3. Reanudación tras suspensión (`PBT_APMRESUMEAUTOMATIC`) → reconexión inmediata.
4. Notificación por pipe tras `provision` → reconexión inmediata.
5. Condición `CLOUD_UNREACHABLE` si no hay conexión durante más de 2 min.

**Dependencies**: HU-WIN-008.

**Technical Considerations**: la política de backoff es una función pura testeable; `PowerRegisterSuspendResumeNotification` o `WM_POWERBROADCAST` en la ventana message-only.

**Tests**: unit de la secuencia de backoff; integración: parar/arrancar el broker; credencial inválida → 5 min (con reloj falso).

**Definition of Done**: DoD común.

---

## HU-WIN-010 — Validación de mensajes entrantes

**Goal**: ningún mensaje malformado u hostil afecta al agente.

**Description**: etapas 1–2 de [Execution Pipeline](../../01-architecture/execution-pipeline.md#etapas).

**Acceptance Criteria**
1. Payload > 64 KiB → descartado antes de parsear.
2. JSON inválido, `v ≠ 1`, topic ajeno, `deviceId ≠ self` o `type` inesperado → descartado con `WARN` (sin respuesta al Cloud).
3. `taskId` legible con `data` inválido → `task.rejected INVALID_PAYLOAD`.
4. Límite de tasa: más de 50 mensajes/s entrantes → se descartan los excedentes y se registra una sola vez por minuto.
5. Ningún mensaje provoca pánico (fuzzing básico en HU-WIN-047).

**Dependencies**: HU-WIN-006, HU-WIN-007.

**Technical Considerations**: profundidad máxima de JSON (`serde_json` con `disable_recursion_limit` desactivado; límite propio si procede).

**Tests**: tabla de mensajes inválidos; test de tamaño límite exacto (64 KiB y 64 KiB + 1).

**Definition of Done**: DoD común.

---

## HU-WIN-011 — Estado de la sesión de Windows

**Goal**: conocer si la PC está bloqueada, inactiva o desconectada.

**Description**: implementar `sessionState` de [Session Model](../../05-windows/session-model.md#estado-de-sesión).

**Acceptance Criteria**
1. `WTSRegisterSessionNotification` en la ventana message-only: lock/unlock y connect/disconnect actualizan el estado.
2. `Idle` por `GetLastInputInfo` con umbral configurable (default 10 min), sondeo cada 30 s.
3. Estado inicial correcto al arrancar con la sesión ya bloqueada (`WTSQuerySessionInformation` `WTSSessionInfoEx`).
4. Cambios publicados en heartbeat (HU-WIN-008) y visibles en `status`.

**Dependencies**: HU-WIN-004, HU-WIN-008.

**Technical Considerations**: interfaz `SessionApi` en `jarvis-windows` con fake para tests del resto del sistema.

**Tests**: integración Windows manual guiada (bloquear con `LockWorkStation`, verificar evento); unit con fake.

**Definition of Done**: DoD común.
