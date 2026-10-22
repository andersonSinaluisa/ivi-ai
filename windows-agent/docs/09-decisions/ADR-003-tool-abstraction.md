# ADR-003: Abstracción de Tool con contrato explícito

## Status

Accepted, 2026-09-19.

## Context

El agente necesita un único punto donde aplicar validación, riesgo, permisos, locks, timeouts, cancelación, verificación y formato de resultado, válido tanto para tareas del Cloud como para el futuro Agent Core y para providers externos.

## Decision

Toda capacidad es una **Tool** con descriptor estático (nombre, versión, riesgo, permisos, esquemas JSON, recursos, timeouts, cancelación, verificación, sensibilidad) y cuatro operaciones: `validate`, `risk`, `execute`, `verify`. El Tool Host ejecuta un pipeline común. Resultado estándar `ToolResult` (`jarvis.tool-result/1`). Detalle en [Tool Contract](../03-tools/tool-contract.md).

Tools builtin compiladas en el binario; providers externos solo fuera de proceso (named pipe). Sin plugins DLL.

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Handlers ad hoc por TaskType | Duplicaría seguridad y verificación en cada handler |
| Plugins DLL dinámicos | Código arbitrario en el proceso del agente, mismo token y memoria |
| Adoptar MCP como protocolo interno | MCP es un buen **formato de exposición** (P10), pero no define riesgo, verificación ni idempotencia; se mantiene compatibilidad de forma (`inputSchema`) sin depender de él |

## Consequences

- ✅ Seguridad y verificación implementadas una vez.
- ✅ Descriptores reutilizables por un LLM.
- ⚠️ Cada tool requiere más declaración inicial (esquemas, verificación); es deliberado.
