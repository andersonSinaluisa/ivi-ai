# EPIC-10 — Simulators

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Los simuladores son **devices reales a nivel de protocolo**: usan TLS, credenciales emitidas por la API, los mismos topics y envelopes. No contienen atajos (no leen la BD, no usan credenciales del Cloud).

---

## HU-047 — Paquete `Jarvis.Contracts` del protocolo

- **Objetivo:** tipos compartidos del protocolo MQTT y DTOs REST.
- **Descripción:** records para el envelope y cada `data` de [MQTT Protocol](../../03-protocols/mqtt-protocol.md); nombres de topics como builders (`Topics.Down.Tasks(deviceId)`); constantes de tipos de mensaje y códigos de error; `JsonSerializerContext` con source generation; DTOs REST.
- **Criterios de aceptación:**
  1. Serializar/deserializar cada ejemplo JSON de [MQTT Protocol](../../03-protocols/mqtt-protocol.md) produce un objeto equivalente (tests con los ejemplos copiados).
  2. Campos desconocidos se ignoran; campos opcionales nulos se omiten al serializar.
  3. Sin dependencias a otros proyectos de la solución.
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** este paquete **no** es la especificación normativa; lo es el documento. Si difieren, se corrige el código.
- **Pruebas esperadas:** unit tests de round-trip.
- **Definition of Done:** DoD común.

---

## HU-048 — Windows Simulator

- **Objetivo:** device ejecutor simulado que cumple las obligaciones W1-W10 del [Windows Agent Contract](../../07-integrations/windows-agent-contract.md).
- **Descripción:** `jarvis-sim windows --device-id … --secret-file … --host …`; inbox/outbox persistidos en SQLite local; `supportedTaskTypes = ["OPEN_CHROME"]`; ejecución simulada con demora configurable (default 1 s) que devuelve `{ "outcome": "SUCCESS", "message": "Chrome opened (simulated)" }`; flags de prueba: `--drop-acks N`, `--disconnect-before-ack`, `--crash-after-start`, `--pause-heartbeats SECONDS`, `--busy`, `--fail-with CODE`, `--slow SECONDS`, `--reject-type`; contador de ejecuciones para verificar no-duplicación.
- **Criterios de aceptación:**
  1. Conecta con TLS y credenciales, envía hello y heartbeats.
  2. Nunca ejecuta dos veces el mismo `taskId` aunque se reciba N veces (verificado con el contador).
  3. Reenvía estados hasta `task.update.ack`.
  4. Tras `--crash-after-start` y reinicio, reporta `Failed EXECUTION_ERROR` sin reejecutar.
  5. Rechaza tareas expiradas y tipos no soportados.
  6. Atiende `task.cancel`.
- **Dependencias:** HU-047.
- **Consideraciones técnicas:** consola .NET en `tools/Jarvis.Simulators`; logs legibles para la demo.
- **Pruebas esperadas:** tests de integración del simulador contra un broker de prueba con un "cloud falso" mínimo, o contra la API real en E2E (HU-050).
- **Definition of Done:** DoD común + guía de uso en README.

---

## HU-049 — ESP32 Simulator

- **Objetivo:** device origen simulado según [ESP32 Contract](../../07-integrations/esp32-contract.md).
- **Descripción:** `jarvis-sim esp32 --device-id … --secret-file … --target <windowsDeviceId>`; comando interactivo "press button" (tecla) y modo `--press-once` para automatización; `idempotencyKey = btn1-{contador persistido}`; reintento del mismo request si no hay `request.accepted` en 10 s; muestra `task.update` como lo haría la pantalla; `task.get` al reconectar para la última tarea.
- **Criterios de aceptación:**
  1. Pulsación → `task.create` → `request.accepted` → muestra estados hasta `Completed` con el mensaje del resultado.
  2. Reintento no crea tareas duplicadas (`duplicate: true`).
  3. Sin grant → muestra `TASK_NOT_GRANTED`.
  4. Tras reconexión, `task.get` muestra el estado final.
- **Dependencias:** HU-047.
- **Consideraciones técnicas:** no usa REST (igual que el ESP32 real).
- **Pruebas esperadas:** idem HU-048.
- **Definition of Done:** DoD común + guía de uso en README.

---

## HU-050 — Escenario E2E del MVP automatizado

- **Objetivo:** demostrar automáticamente la [MVP Definition](../mvp-definition.md).
- **Descripción:** test en `Jarvis.E2ETests` que levanta PostgreSQL + Mosquitto (TLS) + API, crea owner, registra devices Windows y ESP32, crea el grant, arranca ambos simuladores y ejecuta los escenarios E2E-01 a E2E-10 de [Integration Scenarios](../../10-testing/integration-scenarios.md). Script manual equivalente documentado para la demo contra el VPS.
- **Criterios de aceptación:**
  1. Todos los escenarios verdes en CI en < 5 min.
  2. Para el caso principal, la consulta `GET /api/v1/events?correlationId=` devuelve la historia completa en orden.
- **Dependencias:** HU-048, HU-049, HU-034, HU-037.
- **Consideraciones técnicas:** reloj real con timeouts reducidos por configuración (p. ej. ACK 3 s) para que los escenarios de timeout sean rápidos.
- **Pruebas esperadas:** la propia suite.
- **Definition of Done:** DoD común + [Acceptance Tests](../../10-testing/acceptance-tests.md) marcados.
