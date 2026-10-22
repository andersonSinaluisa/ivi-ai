# ADR-015: Mapeo TaskType ↔ Tool

## Status

Accepted, 2026-09-19.

## Context

El Cloud identifica tareas por `type` en `UPPER_SNAKE_CASE` (`OPEN_APP`, `OPEN_CHROME`) y trata `payload` como objeto opaco; el agente organiza capacidades como tools `snake_case`. El catálogo inicial del Cloud y el ESP32 Simulator usan `OPEN_CHROME`. Hace falta una regla estable que no requiera cambios en el Cloud.

## Decision

- `tool = lower(TaskType)`, relación 1:1 (`OPEN_APP` ↔ `open_app`).
- `payload` = input de la tool, validado por su `inputSchema`; los campos siguen el catálogo del Cloud (`appId`, no `app`).
- Excepciones solo como **alias** declarados en el registro: `OPEN_CHROME` → `open_app { "appId": "chrome" }`. Los alias usan los permisos de la tool destino.
- `task.completed.result` = `ToolResult`.
- `supportedTaskTypes` = tools habilitadas + alias.

Detalle en [Tool Registry](../03-tools/tool-registry.md) y [Task Protocol](../02-cloud-integration/task-protocol.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Un único TaskType `RUN_TOOL { tool, input }` | Rompe el catálogo y la validación de `supportedTaskTypes` del Cloud; requeriría cambio de contrato |
| Tabla de mapeo arbitraria | Innecesaria; la convención cubre todos los casos salvo alias |
| Eliminar `OPEN_CHROME` | Rompería el MVP del Cloud y el simulador |

## Consequences

- ✅ Sin cambios en el Cloud; tipos nuevos = tools nuevas.
- ⚠️ Renombrar una tool requiere alias para compatibilidad.
