# Pruebas

Las pruebas viven junto a cada componente; `scripts/test.ps1` las ejecuta todas.

| Requisito | Dónde se prueba |
|---|---|
| Inicialización de servicios | `src-tauri` `config::tests`, `persistence::tests` (migraciones); `voice-service/tests/test_api.py` (arranque, token, binding) |
| Comunicación con Ollama | `infrastructure/llm/ollama.rs`: servidor simulado (streaming, tools, 404, NDJSON inválido, servidor caído) + `real_ollama_round_trip_with_tools` (real, opcional) |
| Errores del LLM | `ollama.rs` + `orchestrator::llm_errors_are_reported_without_crashing` |
| Registro de herramientas | `tools/registry.rs`, `tools/remote.rs` |
| Validación de permisos | `core/policies`, `orchestrator` (aprobación, rechazo, denegación, timeout) |
| Ciclo del agente | `core/agent/orchestrator.rs` |
| Transcripción | `voice-service/tests/test_stt.py` (modelo simulado; real con `JARVIS_TEST_WHISPER=1`) y `/stt/transcribe` |
| Máquina de estados de voz | `test_state_machine.py`, `test_engine.py` |
| Respuestas habladas | `test_engine.py` (frases, respaldo, caché, interrupción) y `test_tts.py` |
| Persistencia de memoria | `persistence` (migración v1 → v2), `memory/memory_repository.rs`, `semantic_memory.rs` |
| Grafo de conocimiento | `memory/memory_graph.rs` (tipos, procedencia, recorrido acotado, subgrafos) |
| Recuperación con Qwen | `memory/memory_retriever.rs` (ranking, expansión, filtros, límite de tokens, instrucciones inertes) y `orchestrator::vertical_slice_memory_saved_retrieved_graphed_exported_and_imported` |
| Embeddings | `memory/embedding_provider.rs` (Ollama simulado, modelo ausente, respaldo léxico) |
| Exportación / importación | `memory/memory_package.rs` (cifrado, contraseñas, KDF hostil, path traversal), `memory_exporter.rs`, `memory_importer.rs` (conflictos, selección, reembeddings, copias) |
| Eventos visuales y Neural Core | `stores/graphStore.test.ts`, `features/neural/*.test.ts(x)`, `features/memory/MemoryScreen.test.tsx` |
| Cancelación | `executor` (timeout y cancelación), `orchestrator::cancellation_stops_the_run`, `ollama::cancellation_stops_the_request`, `test_engine::test_cancel_during_transcription_discards_result` |
| Reconexión | `tool_api::reconnects_once_when_the_pipe_breaks`, `voiceClient.test.ts`, `test_api::test_websocket_ptt_round_trip_and_reconnect` |
| UI | `apps/desktop/src/**/*.test.ts(x)` |

Los dispositivos y modelos se simulan (`ManualInput`, `NullOutput`, `ScriptedStt`, `SilentTts`, `ScriptedProvider`, un windows-agent falso en memoria y un Ollama simulado con wiremock).
