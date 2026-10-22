# Refactorización Local-First + Cloud Core — progreso

Actualizado: 2026-10-22. Leyenda: ✅ implementado y verificado con pruebas que se ejecutaron · 🟡 implementado, sin
verificar en el entorno real (Windows, GPU, VPS) · ⬜ pendiente · ⛔ no se hizo a propósito (requiere autorización).

Auditoría de partida: [architecture/current-state.md](architecture/current-state.md). Decisiones:
[ADR-013](../api/docs/08-decisions/ADR-013-memory-module-in-cloud-core.md) (memoria como módulo del Cloud Core),
[ADR-014](../api/docs/08-decisions/ADR-014-edge-inference-and-device-embeddings.md) (inferencia y embeddings en el
borde), [ADR-015](../api/docs/08-decisions/ADR-015-sync-versions-and-conflicts.md) (versiones y conflictos),
[ADR-016](../api/docs/08-decisions/ADR-016-worker-and-scheduled-jobs.md) (worker y jobs). Protocolo:
[sync-protocol.md](../api/docs/03-protocols/sync-protocol.md).

## Dónde se probó

Linux, sin Docker, sin GPU, sin Windows y **sin tocar el VPS**. PostgreSQL 16 + pgvector 0.6 y Mosquitto 2.0 nativos
en carpetas temporales. Nada se publicó en git (sin commit ni push).

| Suite | Resultado |
|---|---|
| Cloud Core `dotnet test` (servicios nativos, `JARVIS_TEST_SERVICES=native`) | 226 pasan (0 omitidas) |
| Escritorio Rust `cargo test` | 111 pasan (más 7 escenarios que solo corren con un Cloud Core real; ver abajo) |
| Escritorio `cargo clippy --all-targets -D warnings` | sin advertencias |
| Interfaz `vitest` · `tsc --noEmit` · `vite build` | 40 pasan · OK · OK |
| Servicio de voz `pytest` | 42 pasan (incluye STT/TTS contra `jarvis-stt`/`jarvis-tts` reales con modelos simulados) |
| `jarvis-stt` / `jarvis-tts` (`api/deploy/voice/*/test_server.py`) | 4 + 3 pasan |
| windows-agent `cargo test` (sin cambios en esta fase) | 62 pasan · 4 fallan solo en Linux (rutas `C:\…` en `jarvis-policy::files`), igual que en la auditoría |
| Escritorio ↔ Cloud Core con servicios reales, escenarios A–H (`tests/desktop-cloud/run-local.sh`) | ✅ A, B, C, D, F, G, H (Rust) y E (reinicio de API + worker) |
| Aprovisionamiento PostgreSQL + migraciones + permisos (`tests/deploy/check-provision.sh`) | ✅ (idempotente, base ajena intacta, worker sin acceso a usuarios) |
| Sitio nginx (`nginx -t` 1.24 y límites de cuerpo de `/api/v1/sync/push`) | ✅ 900 KB pasa · 1,1 MB → 413 · otras rutas > 64 KB → 413 |
| nginx `auth_request` en /ollama, /stt, /tts | ✅ sin credencial o inválida → 401; válida → llega al servicio sin el token |

## Fases

| # | Fase | Estado | Dónde |
|---|---|---|---|
| 1 | Auditoría y estado actual | ✅ | `docs/architecture/current-state.md` |
| 2 | Arquitectura objetivo y ADR | ✅ | ADR-013…016 |
| 3 | Contratos versionados (`jarvis.sync/1`) con vectores compartidos .NET ↔ Rust | ✅ | `api/docs/03-protocols/sync-vectors.json` (copia idéntica en `assistant/packages/agent-protocol/schemas/`, comprobada por prueba) |
| 4 | Memory Service + Knowledge Graph en PostgreSQL (esquema `memory`, versiones, tombstones, pgvector opcional) | ✅ | `api/src/*/Memory`, `*/Sync` |
| 5 | Sync Service: outbox, push/pull incremental, idempotencia, fusión a tres bandas, conflictos | ✅ | Cloud: `SyncPushHandler`, `SyncHandlers`; escritorio: `src-tauri/src/sync/` |
| 6 | Experience Engine y Skill Registry (habilidades nunca se ejecutan solas; propuestas que el owner acepta) | ✅ | `MemoryHandlers.cs`, jobs `memory.consolidate` |
| 7 | Scheduler + `jarvis-worker` (catálogo cerrado, `SKIP LOCKED`, reintentos, timeouts, aprobación) | ✅ | `api/src/Jarvis.Worker` |
| 8 | Inference Gateway en el escritorio (Ollama local, Ollama remoto, API compatible; local/remoto/auto) | ✅ con servidores simulados · 🟡 sin Ollama ni proveedor real | `src-tauri/src/infrastructure/llm/gateway.rs` |
| 9 | Interfaz: Infraestructura, Memoria (conflictos), Modelos, nodos remotos en el Neural Core | ✅ | `apps/desktop/src/features/settings/`, `stores/graphStore.ts` |
| 10 | VibeVoice como proveedor de voz opcional y experimental | Retirado por ADR-017 (voz con Piper en el servidor) | — |
| 11 | Despliegue para Contabo | ✅ archivos y pruebas locales · ⛔ no ejecutado en el VPS | `api/deploy/`, `api/docs/06-infrastructure/contabo-deployment.md` |
| 12 | Escenarios A–G | ✅ | `tests/desktop-cloud/` |

## Escenarios A–G

| | Escenario | Prueba |
|---|---|---|
| A | Un recuerdo creado en la PC A se recupera en la PC B (en local tras el pull y en la búsqueda del Cloud) | `scenario_a_…` (dos PCs con SQLite propio contra el Cloud real) |
| B | Trabajar sin Internet y sincronizar después (outbox) | `scenario_b_…` |
| C | El mismo recuerdo editado en dos PCs: campos distintos se fusionan; el mismo campo queda en conflicto hasta resolverlo; nada se pierde | `scenario_c_…` |
| D | Cambiar el modelo de embeddings sin perder recuerdos; otra PC con el mismo modelo recibe los vectores sin recalcular | `scenario_d_…` |
| E | Reiniciar el Cloud Core sin perder tareas persistentes; el worker ejecuta jobs y pasa su healthcheck | `run-local.sh` (procesos reales reiniciados) |
| F | Un device revocado deja de sincronizar y Jarvis sigue funcionando en local | `scenario_f_…` |
| G | Restaurar la memoria desde un respaldo validado en una PC nueva y volver a sincronizar sin duplicados | `scenario_g_…` |
| H | La credencial del equipo emparejado abre los servicios del servidor (lo que valida nginx); sin emparejar no hay credencial | `scenario_h_…` |

## Todo por el servidor (ADR-017, 22-oct-2026)

Por decisión del owner, la PC ya no ejecuta modelos: Qwen, embeddings, transcripción y voz están en el VPS, detrás de
nginx con la credencial del device (`auth_request` → `GET /api/v1/access/check`). Sin conexión no hay chat ni voz.

| Pieza | Dónde | Estado |
|---|---|---|
| `/api/v1/access/check` | Cloud Core | ✅ prueba de integración (usuario, device, anónimo, token falso, revocación) |
| Rutas `/ollama/`, `/stt/`, `/tts/` con `auth_request` | `api/deploy/nginx/jarvis-site.conf.template` | ✅ nginx 1.24 local: 401 sin credencial o con una inválida; el token no llega al servicio |
| `jarvis-stt` (faster-whisper) y `jarvis-tts` (Piper) | `api/deploy/voice/` | ✅ pruebas con modelos simulados · 🟡 imágenes Docker y modelos reales sin probar aquí |
| Gateway y embeddings con la credencial del device | `assistant/.../src-tauri` | ✅ wiremock + escenario H contra el Cloud Core real |
| Servicio de voz: STT/TTS remotos, credencial en memoria | `assistant/services/voice-service` | ✅ contra los servidores reales (modelos simulados) |
| Retirados: Ollama local, Whisper local, OpenVoice, VibeVoice, perfiles de voz | escritorio y servicio de voz | ✅ |

## Lo que queda por verificar fuera de este entorno

- Compilar y ejecutar el escritorio en **Windows** (Credential Manager con `keyring`, WebView2, SAPI).
- Qwen, faster-whisper y Piper reales en el VPS (las imágenes de `deploy/voice` no se construyeron aquí).
- **Despliegue en el VPS**: primero `api/deploy/scripts/inspect-vps.sh` (solo lectura) con tu autorización; después
  `deploy.sh` siguiendo [contabo-deployment.md](../api/docs/06-infrastructure/contabo-deployment.md).
- Construir las imágenes Docker (`Dockerfile`, `Dockerfile.worker`): aquí no hay Docker. El `dotnet publish` del worker
  con el mismo contexto que la imagen sí se ejecutó.

## Hallazgos en el código previo (corregidos)

- La solución .NET no compilaba con el SDK 10.0.402 (aviso GHSA-334f-vvp4-7g6p en SignalR 10.0.12 + avisos como
  errores): paquetes 10.0.12 → 10.0.13.
- Carrera en las pruebas: el certificado de prueba podía caducar después que su CA.
- `deploy.sh`: los secretos montados por Compose quedaban `root 0400` y las imágenes .NET corren como UID 1654, así que
  la API no podía leerlos. Ahora pasan a `root:1654 0440` (solo los que usan la API y el worker). **Sin probar con
  Docker real.**
- Plantilla de nginx: `http2 on;` no existe en nginx < 1.25.1 (Ubuntu 22.04/24.04); `deploy.sh` lo adapta.
