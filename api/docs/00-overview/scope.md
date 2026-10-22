# Alcance de la Fase 1 — Jarvis Cloud Core

## Objetivo

Construir el backend en el VPS que permita a dispositivos autenticados crear tareas estructuradas dirigidas a otros dispositivos del mismo owner, entregarlas de forma confiable, rastrear su ejecución y devolver el resultado al origen, con presencia, auditoría y notificaciones en tiempo real.

## Vertical slice del MVP

```text
ESP32 Simulator ──MQTT/TLS──► Jarvis Cloud ──MQTT/TLS──► Windows Simulator
                                                              │ "OPEN_CHROME" (simulado)
ESP32 Simulator ◄──MQTT/TLS── Jarvis Cloud ◄──MQTT/TLS────────┘ Completed
```

Criterios exactos: [MVP Definition](../09-backlog/mvp-definition.md).

## Dentro del alcance (IN)

| Área | Incluye |
|---|---|
| Identidad | Usuario owner (multi-user preparado, sin registro público), login, JWT + refresh tokens, bootstrap del primer owner por comando. |
| Devices | Registro por el owner, credencial mostrada una sola vez, listado, detalle, rename, disable/enable, revoke, rotación de credencial. |
| Autenticación de dispositivos | Secreto por dispositivo para MQTT (Mosquitto Dynamic Security) y para REST (token de dispositivo de vida corta). |
| Autorización | Scoping por owner, permisos por dispositivo (*task grants*), ACL MQTT por dispositivo. |
| Tasks | Creación (REST y MQTT), idempotencia, dispatch, ACK, estados, progreso, resultado, cancelación, expiración, redelivery, timeouts. |
| Presence | hello, heartbeat, LWT, goodbye, sweeper Online/Stale/Offline, Busy reportado, Disabled. |
| Events | Event log operacional append-only y API de consulta con cursor. |
| Realtime | Hub SignalR para clientes UI (sin UI todavía), catch-up vía REST. |
| Seguridad | TLS, rate limiting, lockout, headers, redacción de logs, roles de BD de mínimo privilegio. |
| Observabilidad | Logs JSON estructurados, correlation id, métricas, `/health/live`, `/health/ready`. |
| Infraestructura | Docker Compose con nginx, jarvis-api, postgres, mosquitto; Let's Encrypt; backups. |
| Simuladores | Windows Simulator y ESP32 Simulator (consolas .NET) que implementan el protocolo real. |

## Fuera del alcance (OUT)

- Windows Agent real, Claude CLI, cualquier LLM, control real de Windows.
- Firmware ESP32, apps mobile, frontend web, UI Tauri.
- Voz, memoria de largo plazo, context engine.
- Flujo de aprobación completo (el estado `WaitingForApproval` y sus mensajes están **diseñados** y el Cloud los acepta; el endpoint de aprobación remota es post-MVP, ver [HU-057](../09-backlog/stories/EPIC-05-task-management.md)).
- Firma end-to-end de tareas (diseñada como evolución, ver [Security Architecture](../05-security/security-architecture.md#evolución-firma-end-to-end-de-tareas)).
- Registro público de usuarios, OIDC externo, multi-tenant real con organizaciones.
- Alta disponibilidad, múltiples instancias de la API, Redis, Kubernetes.
- Almacenamiento de archivos/artefactos grandes.

## Supuestos

| # | Supuesto | Impacto si es falso |
|---|---|---|
| S1 | Un único VPS Linux (Ubuntu Server 24.04 LTS) con Docker Engine y Compose v2. | Ajustar [Deployment](../06-infrastructure/deployment-architecture.md). |
| S2 | Existe un dominio propio con DNS controlable (placeholder `${JARVIS_DOMAIN}`). | Sin dominio no hay Let's Encrypt; habría que usar CA propia. |
| S3 | Decenas de dispositivos como máximo y decenas de tareas por minuto como pico. | Por encima de ~1000 dispositivos revisar presencia en BD y ADR-008. |
| S4 | Un solo owner real al inicio; el modelo soporta varios. | — |
| S5 | Los dispositivos tienen reloj sincronizado (NTP/SNTP) con error < 60 s. | Afecta expiración y ventana anti-replay. |
| S6 | El puerto 8883/TCP puede abrirse en el VPS y es alcanzable desde las redes de los dispositivos. | Alternativa: MQTT sobre WebSocket por 443 (documentado como plan B en [MQTT Protocol](../03-protocols/mqtt-protocol.md)). |

## Restricciones

- Recursos: 4 vCPU, 8 GB RAM, 100 GB SSD. Presupuesto de memoria en [Docker Strategy](../06-infrastructure/docker-strategy.md).
- Sin servicios gestionados obligatorios: todo debe poder correr en el VPS.
- Solo nginx (80/443) y Mosquitto (8883) expuestos a Internet.
