# ADR-010 — LLM independence

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Jarvis usará Claude CLI inicialmente como motor de razonamiento en el Windows Agent, pero debe poder cambiar de proveedor (Anthropic, OpenAI, modelos locales) sin rediseñar. El Cloud es la pieza más expuesta; acoplarlo a un proveedor aumentaría superficie y costo de cambio.

## Decision

1. **El Cloud no conoce ni transporta conceptos de LLM**: no hay campos de modelo, prompt, tokens, tool calls ni proveedor en el esquema de BD, en REST, en MQTT ni en SignalR.
2. El razonamiento ocurre **en el device ejecutor** (Windows Agent) detrás de una interfaz `IReasoningProvider`; Claude CLI es un adaptador.
3. Las instrucciones en lenguaje natural viajan como una tarea más (`ASSISTANT_REQUEST` con `payload.text`); su respuesta viaja en `result`. Para el Cloud son JSON opaco.
4. El Cloud nunca ofrece ejecución genérica (`/execute-shell` o equivalente); solo tareas estructuradas cuyo significado define el device.
5. El Cloud no almacena credenciales de proveedores de IA.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Razonamiento en el Cloud (llamar a la API del LLM desde el VPS) | El VPS tendría credenciales de IA y decidiría acciones → un compromiso del VPS equivaldría a control remoto con "inteligencia"; además acopla el Cloud al proveedor. Podría revisarse para respuestas conversacionales sin acciones, con ADR nuevo. |
| Tipos de tarea específicos por proveedor | Acoplamiento directo. |
| Campos opcionales `model`/`provider` "para métricas" | Filtra detalles internos al Cloud sin necesidad; si se quieren métricas, van dentro de `result`. |

## Consequences

- (+) Cambiar de proveedor afecta solo al Windows Agent.
- (+) El Cloud sigue siendo pequeño y auditable.
- (−) Sin métricas centrales de uso del LLM (se pueden reportar dentro de `result` si se desea, sin esquema en el Cloud).
- Verificación: la revisión de consistencia de la documentación comprueba que ningún documento normativo del Cloud depende de Claude ([docs/README.md](../README.md)).
