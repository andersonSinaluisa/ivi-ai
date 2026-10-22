# ADR-014: Boundary con el Agent Core y BrainProvider

## Status

Accepted, 2026-09-19. Implementación de la Local Tool API en P10.

## Context

El ecosistema tendrá un `jarvis-agent-core` que usará un LLM (Claude CLI, OpenAI, modelo local) para convertir objetivos en llamadas a tools. Las mismas tools deben poder invocarse desde tareas remotas del Cloud y desde el Agent Core local sin duplicar implementación, y el Windows Agent no debe contener razonamiento.

## Decision

- El Windows Agent expone las tools por dos frontends sobre **un único pipeline**: Cloud Task Adapter (MQTT) y Local Tool API (named pipe, JSON-RPC 2.0, `tools/list` y `tools/call`, forma compatible con MCP).
- `local:agent-core` es un principal con su propia política; R3 sigue requiriendo aprobación humana.
- El Windows Agent **nunca** depende de `BrainProvider` ni de sus implementaciones (`ClaudeCliProvider`, `OpenAIProvider`, `LocalModelProvider`): ni crates, ni procesos, ni API keys de LLM. La dependencia es solo Agent Core → Local Tool API.
- Las tools se pueden ejecutar sin ningún LLM.

Detalle en [Agent Core Integration](../08-future/agent-core-integration.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Agent Core dentro del Windows Agent | Mezcla razonamiento no determinista con la autoridad de seguridad; API keys de LLM en el proceso que controla la PC |
| Agent Core que envía tareas vía Cloud | Latencia y dependencia de Internet para acciones locales; aceptable como modo adicional, no como único |
| Agent Core que llama Win32 directamente | Duplica tools sin política ni verificación |

## Consequences

- ✅ Cambiar de LLM no toca el Windows Agent; el Windows Agent es auditable sin conocimiento de IA.
- ✅ El MVP ya modela `Invocation`, `ToolResult`, descriptores JSON Schema y principals `local:*`.
- ⚠️ Un contrato más que versionar (Local Tool API).
