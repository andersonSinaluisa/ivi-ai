# ADR-A01: Agent core sobre la Local Tool API del windows-agent

## Estado

Aceptado, 2026-10-22.

## Contexto

El encargo pedía crear desde cero un ToolRegistry, un PolicyEngine y herramientas de Windows (abrir y cerrar apps, procesos, ventanas, archivos) dentro de la app Tauri. En `ivi/windows-agent` ya existe un agente Rust que implementa esas herramientas con política R0–R4, locks, verificación, auditoría y UI Automation. Además expone una Local Tool API (named pipe, JSON-RPC) pensada justo para un agent core con LLM (ADR-014 de ese proyecto), y ese ADR descarta que el agent core llame a Win32 directamente.

## Decisión

- La app de escritorio es ese **agent core**: contiene el LLM, el ciclo del agente, la memoria y la UI.
- Las acciones sobre Windows se ejecutan **solo** por la Local Tool API, como el principal `local:agent-core`.
- La app mantiene su propio `ToolRegistry` y `PolicyEngine` como **primera barrera**. Ese registro tiene herramientas locales de solo lectura y proxies de las remotas; la política decide si se pide confirmación al usuario en la UI. El windows-agent aplica después la suya: dos barreras independientes.
- Sin windows-agent, Jarvis sigue funcionando con las herramientas locales de solo lectura.

## Consecuencias

- No se duplica la implementación de las herramientas ni su política.
- `open_app`, `close_app`, `list_processes`, `read_file`, `list_directory`, etc. dependen de que el windows-agent esté corriendo con `[tool_api] enabled = true`.
- Hay un contrato más que versionar (JSON-RPC y `jarvis.tool-result/1`); el cliente reconecta si el pipe se cae.
