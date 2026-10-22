# Progreso

Actualizado: 2026-10-22. Leyenda:

- ✅ **Verificado**: implementado y cubierto por pruebas que se ejecutaron y pasaron.
- 🟡 **Implementado, sin verificar**: el código existe, pero no se ejecutó en Windows ni con el modelo o dispositivo real.
- ⬜ **Pendiente**.

## Dónde se probó

Todo se ejecutó en un entorno **Linux** de desarrollo, sin GPU, sin micrófono, sin Ollama y sin acceso a Hugging Face. Lo exclusivo de Windows (named pipe, voz SAPI, compilación con MSVC y WebView2) **no se compiló ni se ejecutó**: se revisó a mano. Hay que validarlo en tu PC con `scripts\test.ps1` y `scripts\dev.ps1`.

| Suite | Resultado |
|---|---|
| Rust `cargo test` (Linux) | 110 pasan (incluye 1 real que se salta sola: `real_ollama_round_trip_with_tools`, se activa con `JARVIS_TEST_OLLAMA=1`) |
| `cargo clippy --all-targets` | sin advertencias |
| Interfaz `vitest` | 37 pasan |
| Interfaz `tsc --noEmit` + `vite build` | OK (Three.js va en un fragmento aparte, cargado solo en la vista 3D) |
| Neural Core 3D en Chromium sin GPU (SwiftShader, Playwright) | Se dibuja; la selección de un nodo con el ratón funciona; 2D y bajo consumo también |
| Voz `pytest` (Python 3.11) | 45 pasan · 1 real omitida (Whisper, `JARVIS_TEST_WHISPER=1`) |
| Arranque real del servicio de voz (sin audio ni modelos) | Arranca, exige token y reporta cada dependencia ausente |
| `cargo check` para Windows | **No ejecutado**: no se pudo descargar la librería estándar de Rust para Windows en este entorno |

## Fase 1: Fundación

| Ítem | Estado |
|---|---|
| Monorepo (`apps/`, `services/`, `packages/`, `config/`, `scripts/`, `docs/`, `tests/`) | ✅ |
| Tauri v2 + React 19 + TypeScript + Vite + Tailwind v4 + Zustand | ✅ compila el frontend · 🟡 la ventana Tauri no se abrió en Windows |
| Backend Rust organizado (commands, core, domain, infrastructure, tools) | ✅ |
| Servicio FastAPI ejecutable | ✅ |
| Contratos: `shared-types`, `agent-protocol` (JSON Schema), `tool-schemas` | ✅ |
| Comunicación UI ↔ Rust (comandos + `Channel` en streaming) | 🟡 los tipos coinciden con las pruebas de serialización; falta la prueba E2E en Tauri |
| Comunicación UI ↔ voz (REST + WebSocket con token) | ✅ en el servidor · ✅ cliente con reconexión |
| Scripts de Windows (check-env, setup, dev, test, build) | 🟡 revisados a mano; no ejecutados en PowerShell |

## Fase 2: Cerebro

| Ítem | Estado |
|---|---|
| `LlmProvider` + `OllamaProvider` (`/api/chat`, streaming NDJSON, tool calling, salida estructurada) | ✅ con servidor simulado · 🟡 contra Ollama real |
| Manejo de errores: Ollama caído, modelo no instalado, líneas inválidas, timeout, cancelación | ✅ |
| AgentOrchestrator con 7 fases y componentes independientes | ✅ |
| ToolRegistry (id, nombre, descripción, esquema, riesgo, permisos, timeout, cancelación, auditoría) | ✅ |
| Herramientas locales `get_current_time`, `get_system_info` | ✅ |
| Proxy a la Local Tool API del windows-agent (descubrimiento, llamada, reconexión) | ✅ con un agente simulado en memoria · 🟡 named pipe real |
| Herramientas de Windows del windows-agent: `open_app`, `close_app`, `is_app_running`, `list_running_apps` (equivale a `list_processes`), `get_active_window`, `find_window` / `focus_window` (en lugar de `list_windows`), `read_file`, `list_directory`, `run_command`, `show_notification` y UI Automation (`find_element`, `get_text`, `set_text`, `invoke_element`) | 🟡 llegan por `tools/list` si están habilitadas en su `config.toml`; integración real sin probar |
| `open_url`, `list_windows` con esos nombres | ⬜ no existen en el windows-agent (su ADR-013 trata el navegador aparte) |
| PolicyEngine (READ_ONLY / LOW_RISK / SENSITIVE / DESTRUCTIVE, deny, modos por herramienta, contenido externo) | ✅ |
| Confirmación en la UI | ✅ (prueba de interfaz con backend simulado) |
| Auditoría en SQLite y pantalla de Actividad | ✅ |

## Fase 3: Voz

| Ítem | Estado |
|---|---|
| Máquina de estados (8 estados, transiciones validadas) | ✅ |
| VoiceEngine: pulsar para hablar, VAD, silencio, duración máxima, cancelación, interrupción, recuperación de errores | ✅ con dispositivos simulados |
| No activarse con su propia voz (micrófono ignorado al hablar + enfriamiento) | ✅ |
| Captura de micrófono y reproducción (sounddevice) | 🟡 no probado con hardware |
| Micrófono y altavoz configurables, volumen, activar o desactivar la escucha | ✅ API · 🟡 hardware |
| Eventos de nivel de audio para la visualización | ✅ |
| faster-whisper | ✅ adaptador con modelo simulado · 🟡 modelo real (no se pudo descargar aquí) |
| VAD de Silero al transcribir | 🟡 (`vad_filter=True`, función de faster-whisper) |
| TTS sin clonación: voz de Windows (System.Speech) | 🟡 solo en Windows; no ejecutado |
| Caché de audio, frases por fragmentos, respaldo si falla el motor | ✅ |
| OpenVoice V2 (adaptador + instalador) | 🟡 escrito según su API pública; **no ejecutado** (PyTorch + checkpoints) |
| Perfiles de voz: grabar o importar WAV, consentimiento obligatorio, activo, borrar | ✅ API · 🟡 grabación con micrófono real |
| Pantalla de configuración de voz | ✅ compila · 🟡 sin servicio real conectado |
| Palabra de activación | 🟡 adaptador openWakeWord listo, **desactivado**. Solo existe «hey jarvis» en inglés (licencia no comercial). **No se declara funcional** (docs/voice-wake-word.md) |
| Interrumpir la respuesta (botón «Callar» o pulsar para hablar) | ✅ |
| Español por defecto; otros idiomas con `language` | ✅ configurable · ⬜ la UI solo está en español |

## Fase 4: Memoria — Neural Memory Engine

Corte vertical **verificado** por la prueba `vertical_slice_memory_saved_retrieved_graphed_exported_and_imported` (Qwen simulado): «recuerda que…» → Qwen llama a `save_memory` → el recuerdo se guarda y se enlaza → en otra conversación se recupera y entra en el contexto **antes** de las reglas de seguridad → se registra `MEMORY_RETRIEVED` y sale el evento visual → el nodo y sus relaciones aparecen en `memory_graph` → exportación cifrada → importación en una base vacía con el mismo id.

| Ítem | Estado |
|---|---|
| Módulo `src/memory/` (engine, repository, retriever, graph, embeddings, episódica, semántica, procedimental, experiencias, consolidador, exportador, importador, paquete) | ✅ |
| Migración SQLite v2 (`memories`, `memory_relations`, `memory_sources`, `experiences`, `skills`, `skill_versions`, `memory_events`, `embedding_metadata`) y paso de v1 → v2 sin perder datos | ✅ |
| Grafo tipado con procedencia (usuario, inferida, importada…), confianza y verificación; consultas de vecinos, recorrido y subgrafo | ✅ |
| Relaciones inferidas automáticamente: nunca nacen verificadas | ✅ |
| Recuperación: embedding → similitud → expansión por grafo → filtros → ranking → contexto con límite de tokens → registro de uso | ✅ |
| Los recuerdos no pueden anular las reglas de seguridad (van antes de ellas, saneados: sin bloques de código ni marcadores falsos) | ✅ |
| Embeddings con `qwen3-embedding:0.6b` vía `/api/embed` | ✅ con Ollama simulado · 🟡 **sin probar con el modelo real** |
| Respaldo léxico si no hay modelo de embeddings (la UI lo indica) | ✅ |
| Reembeddings en segundo plano, cancelables | ✅ |
| Experiencias con deduplicación; solo las verificadas se promueven a habilidad, con versiones | ✅ |
| Las habilidades importadas llegan desactivadas y nunca se ejecutan solas | ✅ |
| Exportación cifrada `.jarvismem` (Argon2id + XChaCha20-Poly1305, `manifest.json`, `memories.sqlite`, `skills/`, `attachments/`, `checksums.json`), sin credenciales ni conversaciones | ✅ |
| Importación: contraseña, integridad, versión, migraciones, vista previa, importación selectiva, duplicados, conflictos, reembeddings, copia previa y restauración | ✅ |
| Rutas maliciosas dentro del paquete (`../`) rechazadas | ✅ |
| Eventos visuales tipados Rust → React (`jarvis://visual`) | ✅ reductor probado · 🟡 el puente Tauri real no se ejecutó en Windows |
| Neural Core 3D (React Three Fiber, un draw call por capa, bajo consumo, alternativa 2D) con datos reales del Memory Engine | ✅ en el navegador con backend simulado · 🟡 sin probar dentro de Tauri/WebView2 |
| Pantalla Memoria: explorar, buscar (texto o significado), relaciones, fuentes, editar, borrar, experiencias, habilidades, exportar/importar, copias | ✅ pruebas de componente · 🟡 sin prueba E2E con Tauri |
| Nivel de la voz de salida para animar el núcleo (`output_level`) | ✅ |
| Rendimiento con decenas de miles de recuerdos | 🟡 vista paginada (400 nodos; 150 en bajo consumo) y búsqueda vectorial lineal en caché; no se midió a gran escala |

## Fase 5: Autonomía

| Ítem | Estado |
|---|---|
| Planificación multietapa | 🟡 el modelo encadena llamadas (hasta `max_steps`); no hay un plan explícito revisable antes de ejecutar |
| Verificación de resultados | ✅ básica (usa la del windows-agent si viene) |
| Sistema de permisos | ✅ |
| Aprendizaje de habilidades | 🟡 promoción manual; ⬜ reutilizar habilidades en la planificación |
| Qwen-VL (visión) | ⬜ el contrato `LlmProvider` lo admite; sin implementar |
| PowerShell con políticas, Playwright | ⬜ corresponden al windows-agent (sus ADR-008 y ADR-013) |

## Fase 6: Local-First + Cloud Core

Detalle de toda la refactorización (Cloud Core, despliegue y escenarios A–G): [`../docs/PROGRESS.md`](../docs/PROGRESS.md).

| Ítem | Estado |
|---|---|
| Outbox de sincronización en SQLite (migración 3, triggers solo con la sync activada) | ✅ |
| Cliente del Cloud Core: HTTPS obligatorio (HTTP solo a localhost), token de device renovado, reintentos con espera creciente | ✅ |
| Fusión, conflictos visibles y resolución desde Configuración → Memoria | ✅ (escenario C contra el Cloud real) |
| Recuperación híbrida: local primero + búsqueda en el Cloud con 1,5 s de límite; nodos remotos en el Neural Core | ✅ |
| Credencial del device en el Administrador de credenciales de Windows (`keyring`) | 🟡 en Linux se prueba el almacén de archivo 0600; en Windows sin compilar |
| Inference Gateway (Ollama local / Ollama remoto / API compatible; local, remoto, auto) | ✅ con servidores simulados · 🟡 sin proveedor real |
| Configuración → Infraestructura, Memoria y Modelos | ✅ (vitest + capturas en Chromium) |
| VibeVoice opcional | ✅ adaptador por comando externo · 🟡 modelo real no ejecutado |

## Problemas conocidos

- **Embeddings reales sin probar:** si `qwen3-embedding:0.6b` no está instalado, la memoria usa búsqueda léxica (funciona, pero entiende peor los sinónimos).
- **drei** (ayudas de React Three Fiber) se descartó: ocupaba ~96 MB en `node_modules` y el disco va justo. Se usan los controles de cámara de Three.js directamente.

- **Disco:** había ~5 GB libres. Rust necesita ~2 GB para compilar y `qwen3:4b` ocupa ~2,6 GB. OpenVoice no cabe sin liberar espacio.
- **vitest 3** tiene avisos de seguridad (solo afectan a herramientas de prueba, no a la app; `npm audit --omit=dev` = 0). vitest 4.1.12 no instala con npm 10.9 por una dependencia peer mal publicada; se actualizará cuando se corrija.
- **serde_yaml** está archivado; funciona, pero conviene reemplazarlo más adelante.
- El servicio de voz no lo arranca Tauri: hay que usar `scripts\dev.ps1` o `scripts\start-voice.ps1`.

## Siguiente paso recomendado

1. En tu PC: `scripts\check-env.ps1` → `ollama pull qwen3:4b` → `ollama pull qwen3-embedding:0.6b` → `scripts\setup.ps1` → `scripts\test.ps1` (con `JARVIS_TEST_OLLAMA=1`) → `scripts\dev.ps1`.
2. Corregir lo que salga de compilar en Windows (named pipe, SAPI).
3. Activar la Tool API del windows-agent con una política mínima para `local:agent-core` (solo R0) y probar «¿qué apps tengo abiertas?».
