# Jarvis Cloud Core — Documentación

Fuente de verdad del diseño de la Phase 1. Idioma: español, con términos técnicos en inglés. Términos definidos en el [Glosario](00-overview/glossary.md).

## Mapa

| Carpeta | Documento | Contenido |
|---|---|---|
| 00-overview | [vision.md](00-overview/vision.md) | Visión y principios |
| | [scope.md](00-overview/scope.md) | Alcance, supuestos, restricciones |
| | [glossary.md](00-overview/glossary.md) | Terminología normativa |
| | [roadmap.md](00-overview/roadmap.md) | Fases 0-10 |
| 01-architecture | [system-context.md](01-architecture/system-context.md) | Diagrama de contexto |
| | [cloud-architecture.md](01-architecture/cloud-architecture.md) | Contenedores, background services, análisis del stack |
| | [backend-architecture.md](01-architecture/backend-architecture.md) | Capas y responsabilidades |
| | [data-flow.md](01-architecture/data-flow.md) | Secuencias: registro, tarea, offline, reconexión, cancelación |
| | [trust-boundaries.md](01-architecture/trust-boundaries.md) | Fronteras de confianza |
| 02-domain | [domain-model.md](02-domain/domain-model.md) | Agregados y entidades |
| | [task-state-machine.md](02-domain/task-state-machine.md) | Estados y transiciones de tareas |
| | [device-model.md](02-domain/device-model.md) | Device, estados, capabilities |
| | [event-model.md](02-domain/event-model.md) | Event log y catálogo |
| 03-protocols | [mqtt-protocol.md](03-protocols/mqtt-protocol.md) | Protocolo MQTT v1 (normativo) |
| | [rest-api.md](03-protocols/rest-api.md) | REST API v1 (normativo) |
| | [realtime-protocol.md](03-protocols/realtime-protocol.md) | SignalR |
| | [error-model.md](03-protocols/error-model.md) | Problem Details y códigos |
| 04-data | [database-design.md](04-data/database-design.md) | Tablas, índices, constraints, roles |
| | [erd.md](04-data/erd.md) | ERD |
| | [persistence-strategy.md](04-data/persistence-strategy.md) | Fuente de verdad, fallos, transacciones |
| 05-security | [security-architecture.md](05-security/security-architecture.md) | Controles |
| | [threat-model.md](05-security/threat-model.md) | Amenazas T01-T13 |
| | [device-authentication.md](05-security/device-authentication.md) | Credenciales de devices |
| | [authorization.md](05-security/authorization.md) | Matrices de autorización |
| | [secrets-management.md](05-security/secrets-management.md) | Inventario y manejo de secretos |
| 06-infrastructure | [deployment-architecture.md](06-infrastructure/deployment-architecture.md) | Despliegue en el VPS |
| | [docker-strategy.md](06-infrastructure/docker-strategy.md) | Compose futuro |
| | [nginx.md](06-infrastructure/nginx.md) | Proxy, TLS, Let's Encrypt |
| | [networking.md](06-infrastructure/networking.md) | Puertos y redes |
| | [observability.md](06-infrastructure/observability.md) | Logs, métricas, health, alertas |
| | [runbooks.md](06-infrastructure/runbooks.md) | VPS, certificados, despliegue, backups, restauración |
| | [backup-recovery.md](06-infrastructure/backup-recovery.md) | Backups y restauración |
| 07-integrations | [windows-agent-contract.md](07-integrations/windows-agent-contract.md) | Obligaciones del agente Windows |
| | [esp32-contract.md](07-integrations/esp32-contract.md) | ESP32 |
| | [future-mobile-contract.md](07-integrations/future-mobile-contract.md) | Mobile / Web |
| 08-decisions | [README.md](08-decisions/README.md) | ADR-001 … ADR-011 |
| 09-backlog | [epics.md](09-backlog/epics.md) · [user-stories.md](09-backlog/user-stories.md) · [stories/](09-backlog/stories/) · [mvp-definition.md](09-backlog/mvp-definition.md) · [implementation-status.md](09-backlog/implementation-status.md) | Backlog y estado de implementación |
| 10-testing | [testing-strategy.md](10-testing/testing-strategy.md) · [integration-scenarios.md](10-testing/integration-scenarios.md) · [acceptance-tests.md](10-testing/acceptance-tests.md) | Pruebas |

## Jerarquía normativa

Si dos documentos se contradicen, prevalece en este orden:

1. ADRs aceptados ([08-decisions](08-decisions/README.md))
2. Contratos: [MQTT Protocol](03-protocols/mqtt-protocol.md), [REST API](03-protocols/rest-api.md), [Task State Machine](02-domain/task-state-machine.md), [Database Design](04-data/database-design.md)
3. Resto de documentos
4. Backlog (las HU refinan, no redefinen)

La contradicción se corrige en el documento de menor rango en el mismo cambio que la detecta.

## Decisiones pendientes antes de implementar

| # | Decisión | Bloquea | Recomendación |
|---|---|---|---|
| D1 | Dominio definitivo (`${JARVIS_DOMAIN}`) y DNS | HU-052, HU-053 (no bloquea el desarrollo local) | Un subdominio dedicado, p. ej. `jarvis.<tu-dominio>` |
| D2 | Proveedor del VPS y si ofrece firewall de red | HU-054 | Cualquiera con IPv4 fija y firewall de red |
| D3 | ~~Confirmar .NET 10 LTS en lugar de .NET 8~~ | — | **Resuelta**: implementado en .NET 10 |
| D4 | ~~Confirmar Mosquitto + Dynamic Security~~ | — | **Resuelta**: implementado con Mosquitto 2.0.22 + dynsec (rol por device) |
| D5 | Almacenamiento externo de backups | HU-055 | Bucket S3-compatible con credencial de solo escritura |
| D6 | Canal de alertas | HU-056 | Email del owner |
| D7 | Retenciones (eventos 180 días, tokens 30 días) | HU-040 | Aceptar defaults |
| D8 | ~~Estructura de repositorio~~ | — | **Resuelta**: monorepo en `api/` (Cloud + simuladores + pruebas + despliegue) |
| D9 | Lenguaje del futuro Windows Agent (C# o Rust) | No bloquea Phase 1 | Decidir al iniciar Phase 2 |

## Registro de revisión de consistencia

Revisión realizada al cerrar Phase 0 (2026-09-19). Verificado:

- Estados de Task idénticos en dominio, BD (`CHECK`), REST, MQTT, eventos, realtime y backlog: `Pending`, `Dispatched`, `Delivered`, `Running`, `WaitingForApproval`, `Completed`, `Failed`, `Cancelled`, `Expired`. `Queued` fue reemplazado deliberadamente por `Dispatched`.
- Topics MQTT idénticos en protocolo, ACL, contratos de integración y backlog: `jarvis/v1/devices/{deviceId}/up/{tasks|requests|presence|events}` y `…/down/{tasks|notifications|control}`.
- Endpoints REST idénticos en REST API, autorización, backlog y pruebas. `POST /api/devices/register` fue reemplazado deliberadamente por `POST /api/v1/devices`.
- Nombres de eventos (`UPPER_SNAKE_CASE`) y notificaciones (`lower.dot`) con mapeo único en [Event Model](02-domain/event-model.md).
- Ningún documento normativo del Cloud depende de Claude u otro proveedor; Claude solo aparece como implementación futura dentro del Windows Agent.
