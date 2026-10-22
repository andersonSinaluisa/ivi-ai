# MVP Definition — JARVIS CLOUD MVP COMPLETE

El MVP está completo **solo si todas** las condiciones siguientes son verdaderas y verificadas. No hay cumplimiento parcial.

## 1. Demostración principal (en el VPS de producción)

```text
ESP32 Simulator ──► Jarvis Cloud ──► Windows Simulator ──► SUCCESS ──► Jarvis Cloud ──► ESP32 Simulator
```

Procedimiento de demo:

1. El owner (creado con `bootstrap-owner`) hace login por REST.
2. Registra `esp32-sim` (tipo `ESP32`) y `windows-sim` (tipo `Windows`); guarda cada secreto.
3. Crea el grant `(esp32-sim, windows-sim, OPEN_CHROME)`.
4. Arranca ambos simuladores desde una máquina **fuera** del VPS, conectando a `${JARVIS_DOMAIN}:8883` con TLS válido.
5. `GET /api/v1/devices` muestra ambos `Online`.
6. Se pulsa el botón en el ESP32 Simulator.
7. El ESP32 Simulator muestra `request.accepted`, luego `Running` y `Completed` con `Chrome opened (simulated)` en menos de 5 s.
8. `GET /api/v1/tasks/{id}` muestra `Completed`, `requestedBy = esp32-sim`, `target = windows-sim`, las cinco marcas de tiempo y el `result`.
9. `GET /api/v1/events?correlationId={id}` muestra en orden: `TASK_CREATED`, `TASK_DISPATCHED`, `TASK_DELIVERED`, `TASK_STARTED`, `TASK_COMPLETED`, todos con el mismo `correlationId`.
10. Un cliente SignalR conectado como el owner recibió `task.created` … `task.completed`.

## 2. Capacidades obligatorias

| Área | Criterio verificable | Escenario |
|---|---|---|
| Autenticación | Users con JWT + refresh; devices con credencial propia en MQTT y REST; conexión anónima MQTT rechazada | AT-01..AT-04 |
| Persistencia | Reiniciar `jarvis-api` y `mosquitto` durante una tarea `Pending` no la pierde; se entrega al volver | E2E-10 |
| MQTT | Topics, envelope, QoS, ACL según [MQTT Protocol](../03-protocols/mqtt-protocol.md); un device no puede leer ni escribir topics de otro | E2E-05, AT-05 |
| Tasks | Estados y transiciones según [Task State Machine](../02-domain/task-state-machine.md); idempotencia de creación; redelivery sin doble ejecución; expiración; cancelación | E2E-01..E2E-04, E2E-06, E2E-09 |
| Events | Toda transición y operación administrativa tiene evento; historia reconstruible por `taskId` y `correlationId` | E2E-01 |
| Presence | Online / Stale / Offline / Busy / Disabled correctos; LWT de sesión vieja ignorado | E2E-08 |
| Result | El `result` llega al ESP32 Simulator y queda en PostgreSQL | E2E-01 |
| Correlation | Mismo `correlationId` en REST, MQTT, eventos y logs | E2E-01 |
| Seguridad básica | TLS válido en 443 y 8883; solo 22/80/443/8883 abiertos; grants obligatorios; revocación inmediata; rate limiting; sin secretos en logs; aislamiento entre owners | E2E-05, E2E-07, AT-06..AT-12 |

## 3. Calidad y operación

- Todas las HU Must (HU-001 … HU-056 salvo las marcadas Should) cumplen su DoD.
- CI verde: unit, integración y E2E ([Testing Strategy](../10-testing/testing-strategy.md)).
- Backup diario funcionando y **una restauración de prueba exitosa** documentada.
- Runbooks: despliegue, emisión/renovación de certificados, bootstrap de dynsec, restauración.
- Ningún documento ni código del Cloud menciona un proveedor/modelo de IA como dependencia ([ADR-010](../08-decisions/ADR-010-llm-independence.md)).

## 4. Explícitamente NO requerido

LLM, Windows Agent real, firmware, UI, aprobación remota (HU-057), firma end-to-end, Redis, métricas centralizadas, MFA, multi-instancia.

## Declaración

Cuando todo lo anterior esté verificado, se registra en `docs/00-overview/roadmap.md` la fecha de cierre de Phase 1 y se considera **JARVIS CLOUD MVP COMPLETE**.
