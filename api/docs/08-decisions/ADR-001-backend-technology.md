# ADR-001 — Backend technology

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Se necesita un backend para REST, WebSocket (SignalR), cliente MQTT persistente, background services y PostgreSQL, en un VPS de 4 vCPU / 8 GB, mantenido por una persona, con posible Windows Agent futuro en C#. La propuesta inicial era .NET 8 + ASP.NET Core Web API + EF Core + Clean Architecture.

.NET 8 LTS termina soporte el 10-nov-2026, dos meses después de la fecha de este ADR.

## Decision

- **.NET 10 LTS** (soporte hasta nov-2028), C# con nullable habilitado.
- **ASP.NET Core Minimal APIs** con `MapGroup` por recurso (no Controllers).
- **EF Core 10 + Npgsql**, sin repositorios genéricos; SQL explícito para coordinación (`SKIP LOCKED`, updates condicionales).
- **MQTTnet** (MIT) como cliente MQTT en la API y simuladores.
- **SignalR** integrado.
- Estructura: `Jarvis.Domain`, `Jarvis.Application`, `Jarvis.Infrastructure`, `Jarvis.Api` + `Jarvis.Contracts` compartido ([Backend Architecture](../01-architecture/backend-architecture.md)).
- Sin MediatR, sin AutoMapper.

## Alternatives

| Alternativa | Ventajas | Desventajas | Veredicto |
|---|---|---|---|
| .NET 8 | Propuesto; ecosistema maduro | Fin de soporte inminente → migración inmediata | Rechazada |
| Node.js (NestJS/Fastify) | Ecosistema MQTT y WS amplio | Tipado más débil en runtime; menos alineado con un Windows Agent en C# | Rechazada |
| Go | Binario pequeño, gran concurrencia, poco consumo | Más código manual (ORM, validación, auth); sin reutilización de contratos con un agente C# | Rechazada |
| Rust (axum) | Máximo rendimiento y seguridad de memoria | Productividad menor para CRUD/auth; curva alta | Rechazada para Cloud (posible para agente) |
| Controllers MVC | Familiar | Ceremonia sin beneficio para ~25 endpoints | Rechazada |
| Dapper en lugar de EF | Control total de SQL | Migraciones y mapeos manuales | Rechazada; SQL crudo solo donde aporta |

## Consequences

- (+) Soporte largo; `Guid.CreateVersion7`, validación integrada en Minimal APIs, `TimeProvider`.
- (+) Contratos (`Jarvis.Contracts`) reutilizables por simuladores y por un futuro Windows Agent en C#.
- (+) Un solo proceso ~150-300 MB de RAM, holgado en el VPS.
- (−) Application depende de EF Core (decisión pragmática consciente; tests con PostgreSQL real).
- (−) Si el Windows Agent se escribe en Rust, `Jarvis.Contracts` no se reutiliza; el contrato normativo sigue siendo [MQTT Protocol](../03-protocols/mqtt-protocol.md).
