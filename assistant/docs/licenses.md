# Licencias de código, modelos y checkpoints

Revisión inicial (octubre de 2026). Antes de distribuir Jarvis a terceros, vuelve a revisar cada licencia en su fuente: pueden cambiar.

## Código y librerías

| Componente | Licencia | Notas |
|---|---|---|
| Tauri v2 | MIT / Apache-2.0 | |
| React 19, Zustand, Vite | MIT | |
| Tailwind CSS v4 | MIT | |
| serde, tokio, reqwest, rusqlite, sysinfo, tracing | MIT / Apache-2.0 | |
| serde_yaml 0.9 | MIT / Apache-2.0 | Archivado por su autor; funciona. Reemplazar si aparece un problema |
| SQLite | Dominio público | Incluido por `rusqlite` (bundled) |
| FastAPI, Starlette, Pydantic, Uvicorn | MIT / BSD-3 | |
| NumPy | BSD-3 | |
| sounddevice | MIT | Usa PortAudio (MIT) |
| faster-whisper, CTranslate2 | MIT | En el servidor (`jarvis-stt`) |
| Piper (`piper-tts`) | **GPL-3.0** | En el servidor (`jarvis-tts`), sin modificar y como servicio aparte |
| openWakeWord (código) | Apache-2.0 | |
| Ollama | MIT | |
| Three.js, React Three Fiber | MIT | Neural Core 3D |
| argon2, chacha20poly1305, sha2, tar, tempfile (RustCrypto y otros) | MIT / Apache-2.0 | Paquetes de memoria cifrados |

## Modelos

| Modelo | Licencia | Uso en Jarvis |
|---|---|---|
| Qwen3 (pesos abiertos en Ollama) | Apache-2.0 | Modelo por defecto. Las versiones nuevas de Qwen pueden tener otra licencia (por ejemplo, la *Qwen Community License* en algunos modelos grandes): revisa la ficha del modelo que instales |
| Qwen3-Embedding-0.6B (`qwen3-embedding:0.6b`) | Apache-2.0 | Embeddings de la memoria |
| Whisper (pesos de OpenAI, convertidos por Systran para faster-whisper) | MIT | STT, en el servidor |
| Voz Piper `es_MX-claude-high` (rhasspy/piper-voices) | Ver la ficha de la voz | TTS, en el servidor; revisa la licencia de la voz que elijas |
| Modelos preentrenados de openWakeWord (`hey_jarvis`, …) | **CC BY-NC-SA 4.0** | Solo uso no comercial. Desactivados por defecto |
| Voces de Windows (SAPI) | Licencia de Windows | Se usan en tiempo de ejecución; no se redistribuyen |

## Clonación de voz

Retirada (octubre de 2026, [ADR-017](../../api/docs/08-decisions/ADR-017-server-model-services.md)): Jarvis ya no clona voces.
