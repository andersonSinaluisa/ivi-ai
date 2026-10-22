# @jarvis/agent-protocol

Contratos entre procesos, en JSON Schema. Los tipos de TypeScript equivalentes están en `@jarvis/shared-types`.

| Esquema | Canal | Productor → consumidor |
|---|---|---|
| `schemas/agent-event.schema.json` | `tauri::ipc::Channel` del comando `send_message` | Rust → UI |
| `schemas/voice-ws.schema.json` | `ws://127.0.0.1:8765/ws` | UI ↔ servicio de voz |

El contrato con el windows-agent (JSON-RPC `tools/list` y `tools/call`, `jarvis.tool-result/1`) lo define ese proyecto: `windows-agent/crates/jarvis-agent/src/tool_api.rs`.

## Flujo de voz a acción

1. La UI envía `ptt_start` y luego `ptt_stop`; el servicio responde con `transcript`.
2. La UI envía `thinking` al servicio y llama a `send_message(texto)` en Rust.
3. Rust emite `run_started`, `phase`, `token`, `tool_call`… hasta `final`.
4. La UI envía `speak` con el texto final (sin markdown); el servicio pasa a SPEAKING y vuelve a reposo.
5. Si el agente falla o se cancela, la UI envía `agent_done`.
