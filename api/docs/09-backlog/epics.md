# Epics — Phase 1 Cloud Core

| Epic | Nombre | Objetivo | HUs | Archivo |
|---|---|---|---|---|
| EPIC-01 | Cloud Infrastructure | Base de la solución: proyectos, configuración, BD, logging, errores, health, entorno local | HU-001 … HU-007 | [EPIC-01](stories/EPIC-01-cloud-infrastructure.md) |
| EPIC-02 | Identity & Devices | Users/owners, login, registro y administración de devices | HU-008 … HU-016 | [EPIC-02](stories/EPIC-02-identity-devices.md) |
| EPIC-03 | Device Authentication | Tokens de device y sincronización de credenciales con el broker | HU-017 … HU-019 | [EPIC-03](stories/EPIC-03-device-authentication.md) |
| EPIC-04 | MQTT Communication | Broker configurado y canal MQTT del Cloud | HU-020 … HU-023 | [EPIC-04](stories/EPIC-04-mqtt-communication.md) |
| EPIC-05 | Task Management | Ciclo de vida completo de tareas | HU-024 … HU-034, HU-057 (post-MVP) | [EPIC-05](stories/EPIC-05-task-management.md) |
| EPIC-06 | Presence | Online/Stale/Offline/Busy y reconciliación | HU-035 … HU-037 | [EPIC-06](stories/EPIC-06-presence.md) |
| EPIC-07 | Events & Audit | Event log y consulta | HU-038 … HU-040 | [EPIC-07](stories/EPIC-07-events-audit.md) |
| EPIC-08 | Realtime | Hub SignalR y notificaciones | HU-041 … HU-042 | [EPIC-08](stories/EPIC-08-realtime.md) |
| EPIC-09 | Security | Autorización transversal, rate limiting, hardening, redacción | HU-043 … HU-046 | [EPIC-09](stories/EPIC-09-security.md) |
| EPIC-10 | Simulators | Contratos compartidos, simuladores y E2E | HU-047 … HU-050 | [EPIC-10](stories/EPIC-10-simulators.md) |
| EPIC-11 | Deployment | Imagen, nginx/TLS, compose, VPS, backups, monitoreo | HU-051 … HU-056 | [EPIC-11](stories/EPIC-11-deployment.md) |

Total: **57 HU** (56 del MVP + HU-057 post-MVP).
