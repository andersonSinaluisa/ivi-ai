# Failure Scenarios (Chaos)

Comportamiento esperado ante cada fallo. Automatizados en HU-WIN-044 y HU-WIN-043.

| ID | Escenario | Comportamiento esperado | Estado final | Referencia |
|---|---|---|---|---|
| F-01 | **Cloud offline** (API caída, broker vivo) | Conexión MQTT OK pero sin `hello.ack` → `cloud.reachable = false`; tareas en curso terminan; resultados quedan en outbox; se reenvían cuando la API vuelve (el Cloud usa sesión MQTT persistente, `clean start = false` con session expiry, así que el broker le encola los mensajes QoS 1 mientras la API está caída; además el agente reenvía lo no confirmado) | Sin pérdidas | [Health](../07-observability/health.md) |
| F-02 | **MQTT offline** (broker caído) | Reconexión con backoff 1–60 s; tareas en curso siguen; outbox persiste | Resultado entregado al volver | [Reconnection](../02-cloud-integration/reconnection.md) |
| F-03 | **Internet perdido** | Igual a F-02; `CLOUD_UNREACHABLE` tras 2 min; aprobaciones locales siguen funcionando | Igual | [Reconnection](../02-cloud-integration/reconnection.md#comportamiento-offline) |
| F-04 | **Reinicio de Windows** | Apagado ordenado (`goodbye`), o recuperación al siguiente logon si Windows no dio tiempo | Tareas `Running` → `EXECUTION_INTERRUPTED`; resultados pendientes publicados | [Crash Recovery](../06-state/crash-recovery.md) |
| F-05 | **Crash del agente** | Scheduled Task lo reinicia (≤ 1 min); recuperación de arranque; 3 crashes en 10 min → modo seguro | Sin re-ejecuciones | [Crash Recovery](../06-state/crash-recovery.md#crash-loop) |
| F-06 | **Corte de energía durante una tarea** | `synchronous=FULL` garantiza que lo confirmado está en disco; la tarea en `Running` → `EXECUTION_INTERRUPTED` | El efecto parcial es posible (p. ej. app abierta) y se reporta como interrumpida, no como éxito | [Crash Recovery](../06-state/crash-recovery.md) |
| F-07 | **Tarea completada pero resultado no enviado** | Resultado en outbox; `openTaskIds` lo incluye; reenvío hasta `task.update.ack` | Entregado | [Local State](../06-state/local-state.md#el-caso-crítico) |
| F-08 | **Mensaje MQTT duplicado** | Dedup por `taskId`; se reenvía el último estado | 1 ejecución | [Idempotency](../06-state/idempotency.md) |
| F-09 | **Reloj incorrecto** (±1 h local) | Offset desde `hello.ack.serverTime`; expiraciones con hora corregida; `CLOCK_SKEW` | Sin rechazos falsos por expiración | [Reconnection](../02-cloud-integration/reconnection.md#reloj) |
| F-10 | **SQLite corrupto** | BD renombrada, nueva BD, `STATE_RESET`, rechazo de tareas anteriores | Sin re-ejecuciones; resultados no entregados de la BD corrupta perdidos salvo recuperación manual | [Crash Recovery](../06-state/crash-recovery.md#corrupción-de-sqlite) |

## Adicionales

| ID | Escenario | Comportamiento esperado |
|---|---|---|
| F-11 | Disco lleno | Escrituras fallan → `DB_ERROR`; no se hace `task.ack` de tareas nuevas (el Cloud reintentará); tareas en curso terminan y su resultado se intenta persistir con reintentos |
| F-12 | Tool colgada (app que no responde a `WM_CLOSE`, llamada Win32 bloqueada) | Timeout efectivo → `TOOL_TIMEOUT`; tras gracia → `TOOL_STUCK` |
| F-13 | Suspensión del equipo durante una tarea | Al reanudar, la tarea continúa (reloj monotónico sin sesgo); reconexión inmediata |
| F-14 | Política borrada o corrupta en caliente | Se mantiene la última válida; `WARN` |
| F-15 | Broker entrega un mensaje de otro device (fallo de ACL) | Descartado por `deviceId` / topic; `WARN` |

## Nota sobre F-01

El Cloud mantiene una sesión MQTT persistente (`MqttCloudConnection`: `WithCleanStart(false)` + `SessionExpiryInterval`), por lo que el broker encola para él los mensajes QoS 1 publicados mientras la API está caída, dentro del período de expiración de la sesión. Aun así, el agente **no** da nada por entregado hasta `task.update.ack` y reenvía lo no confirmado cada 60 s y tras cada `hello.ack`: cubre caídas más largas que la expiración de la sesión y pérdidas del propio broker.
