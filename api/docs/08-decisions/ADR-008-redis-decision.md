# ADR-008 — Redis decision

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

La propuesta incluía Redis "cuando realmente aporte valor". Se evaluó cada uso potencial para el MVP (1 instancia de API, decenas de devices, decenas de tareas/min).

## Decision

**No incluir Redis en el MVP.**

| Uso | Evaluación | Solución MVP |
|---|---|---|
| Presence | Decenas de heartbeats/min | Columnas en `devices` (`presence_state`, `last_seen_at`) |
| Deduplication | Ya resuelta por constraints y la máquina de estados | Índice único de `idempotency_key`; regla monótona |
| Locks distribuidos | Una sola instancia; y aun con varias | `FOR UPDATE SKIP LOCKED`, advisory locks de PostgreSQL |
| SignalR backplane | Solo necesario con > 1 instancia | Sin backplane |
| Rate limiting | Una instancia | `System.Threading.RateLimiting` en memoria + nginx |
| Caching | Lecturas baratas; volumen bajo | `IMemoryCache` (estado de principales, 30 s) |

## Alternatives

Incluir Redis desde el inicio: +1 contenedor (~50-100 MB), +1 dependencia de readiness, +1 superficie de ataque, +1 cosa que respaldar o aceptar perder, sin beneficio medible.

## Consequences

- (+) Menos piezas, menos memoria, menos fallos posibles.
- (+) PostgreSQL sigue siendo la única fuente de verdad sin excepciones de caché distribuida.
- (−) Escalar a varias instancias requerirá revisar rate limiting y SignalR.

## Criterios para reintroducir Redis

Cualquiera de:
1. Más de una instancia de `jarvis-api` (SignalR backplane y rate limiting compartido). Alternativa a evaluar primero: backplane con PostgreSQL `LISTEN/NOTIFY`.
2. Escrituras de presencia > 50/s sostenidas (≈ 1500 devices con heartbeat de 30 s).
3. Latencia p95 de endpoints de lectura > 200 ms atribuible a BD tras optimizar consultas.
