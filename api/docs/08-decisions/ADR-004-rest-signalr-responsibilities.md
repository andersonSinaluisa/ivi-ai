# ADR-004 — REST + SignalR responsibilities

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Hay tres canales: REST, SignalR y MQTT. Sin reglas claras, una misma operación podría implementarse de tres formas con autorizaciones distintas.

## Decision

| Canal | Responsabilidad | Clientes |
|---|---|---|
| REST | **Comandos y consultas** de UIs y administración. Única vía para administrar devices y grants. | Owner (UIs, herramientas), devices con REST opcional |
| SignalR | **Solo notificaciones** servidor → cliente derivadas de eventos ya persistidos. Sin métodos que cambien estado. | UIs (web, mobile, Tauri) con identidad User |
| MQTT | **Protocolo de devices**: recibir tareas, reportar estado, crear tareas acotadas por grants, presencia. | Devices |

Reglas:
- SignalR nunca es fuente de verdad; el cliente hace catch-up con `GET /api/v1/events?after=`.
- Las notificaciones SignalR no incluyen `payload`/`result` (se obtienen por REST).
- Crear tareas por REST y por MQTT converge en el **mismo caso de uso** (`CreateTask`) con las mismas validaciones.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| SignalR también para comandos (hub methods) | Duplica autorización y validación; más difícil de probar y versionar. |
| MQTT para UIs (MQTT sobre WebSocket) | Exigiría credenciales MQTT para users y ACL por owner en el broker; peor encaje con JWT. |
| Server-Sent Events en lugar de SignalR | Más simple, pero SignalR ya da reconexión y clientes oficiales en JS/.NET; SSE queda como alternativa si se quisiera eliminar SignalR. |
| Solo polling REST | Latencia y carga innecesarias para UIs. |

## Consequences

- (+) Una sola ruta de escritura por operación.
- (+) Autorización centralizada en Application.
- (−) Los clientes UI deben implementar catch-up (documentado en [Realtime Protocol](../03-protocols/realtime-protocol.md)).
