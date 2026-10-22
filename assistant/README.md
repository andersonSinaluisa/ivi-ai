# Jarvis — asistente personal de escritorio

App de escritorio local-first (Tauri v2 + React 19 + Rust) que conversa con **Qwen vía Ollama**, ejecuta acciones solo a través de herramientas autorizadas y escucha y habla mediante un **servicio de voz local** (FastAPI + faster-whisper).

Es el **agent core** previsto en el [ADR-014 del windows-agent](../windows-agent/docs/09-decisions/ADR-014-agent-core-boundary.md): el razonamiento vive aquí y las acciones sobre Windows pasan por la Local Tool API del windows-agent, con su política, verificación y auditoría.

```text
ivi/
├── api/             Jarvis Cloud Core (.NET) — ya existente
├── windows-agent/   Ejecutor de herramientas en Windows (Rust) — ya existente
└── assistant/       ← este monorepo: app de escritorio, voz y contratos
```

## Qué funciona hoy

Resumen; el detalle con estado verificado o pendiente está en [PROGRESS.md](PROGRESS.md).

- Chat con respuestas de Qwen en streaming, historial local en SQLite y cancelación.
- Ciclo del agente OBSERVE → UNDERSTAND → PLAN → AUTHORIZE → EXECUTE → VERIFY → REMEMBER.
- Herramientas locales de lectura (`get_current_time`, `get_system_info`), más las del windows-agent si está corriendo.
- Permisos por herramienta y por nivel de riesgo, confirmación en la UI y auditoría.
- Voz por **pulsar para hablar** (botón o barra espaciadora): grabación con VAD → faster-whisper → Jarvis → voz de Windows.
- **Memoria neuronal**: recuerdos con fuente y confianza, enlazados en un grafo; Jarvis recupera los relevantes por significado en cada turno. Experiencias que solo se convierten en habilidad si están verificadas.
- **Neural Core**: el grafo de la memoria en 3D (o 2D), reaccionando a lo que Jarvis recuerda, aprende y dice.
- Exportar e importar la memoria en un archivo cifrado con contraseña.
- **Modo híbrido (opcional)**: empareja el equipo con tu Cloud Core y la memoria, las experiencias y las habilidades se sincronizan (con conflictos visibles, nunca sobrescritos). Sin conexión todo sigue funcionando en local.
- **Inference Gateway**: Qwen local por Ollama, un Ollama remoto o una API compatible, en modo local, remoto o automático.
- Siete pantallas: Inicio, Conversación, Tareas, Memoria, Herramientas, Actividad y Configuración.

La palabra de activación «Jarvis» **no** está activa: no existe un modelo probado para «Jarvis» en español. Lee [docs/voice-wake-word.md](docs/voice-wake-word.md).

## Requisitos (Windows 10/11)

| Herramienta | Versión | Para qué |
|---|---|---|
| Node.js | 22 LTS (≥ 20.19) | Interfaz |
| Rust | ≥ 1.85 (`rustup`) | Backend Tauri |
| Visual Studio Build Tools | «Desarrollo para el escritorio con C++» | Compilar Rust en Windows |
| WebView2 | viene con Windows 11 | Ventana de la app |
| Python | 3.11 (`py -3.11`) | Servicio de voz |
| Ollama | reciente | Modelo Qwen local |

**Disco:** unos 2 GB para la primera compilación de Rust, 1 GB para el entorno de voz, 0,15 GB para Whisper `base` y unos 2,6 GB para `qwen3:4b`. OpenVoice V2 suma otros 3–5 GB. Si tu disco va justo, mueve los modelos de Ollama a otra unidad con la variable `OLLAMA_MODELS`.

## Puesta en marcha

Desde `ivi\assistant`, en PowerShell:

```powershell
# 1. Revisa qué falta (solo lee, no instala nada)
powershell -ExecutionPolicy Bypass -File scripts\check-env.ps1

# 2. Modelo de lenguaje y modelo de embeddings para la memoria (~0,6 GB)
ollama pull qwen3:4b
ollama pull qwen3-embedding:0.6b

# 3. Dependencias (npm + entorno Python del servicio de voz)
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1

# 4. Arrancar (servicio de voz en otra ventana + app)
powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
```

La primera compilación de Rust tarda varios minutos. Si falta Ollama, el modelo o el servicio de voz, la app arranca igual y lo indica en Inicio.

### Conectar el windows-agent (opcional)

Para abrir apps, listar procesos o leer archivos, Jarvis usa el windows-agent:

1. En `%LOCALAPPDATA%\Jarvis\Agent\config.toml` pon `[tool_api]` con `enabled = true`.
2. En su `policy.toml`, define qué puede hacer el principal `local:agent-core`. El agente aplica esa política aunque Jarvis la pida, y R3 siempre exige su aprobación local.
3. Ejecuta `jarvis-agent run --console` y pulsa **Habilidades → Reconectar windows-agent**.

### Solo la interfaz, en el navegador

`npm run dev -w apps/desktop` abre `http://localhost:1420` con un backend **simulado**, útil para trabajar la UI sin Rust.

## Memoria

- Dile a Jarvis «recuerda que…» o añade recuerdos en **Memoria → Explorar**. Ahí puedes buscarlos (por texto o por significado), ver sus relaciones y su fuente, editarlos, verificarlos o borrarlos.
- Sin `qwen3-embedding:0.6b` la memoria funciona con búsqueda léxica; al instalarlo, pulsa «Reintentar modelo» y Jarvis recalcula los embeddings en segundo plano.
- **Exportar e importar**: Memoria → Exportar e importar. El archivo `.jarvismem` va cifrado con tu contraseña (sin ella no se puede abrir) y por defecto se guarda en `Documentos\Jarvis`. Antes de importar verás un resumen y Jarvis crea una copia de seguridad que puedes restaurar.

## Modo híbrido (Cloud Core)

- **Configuración → Infraestructura**: escribe la URL HTTPS de tu Cloud Core, tu correo y contraseña de owner y el nombre del equipo. La contraseña solo se usa para registrar el equipo; la credencial del device se guarda en el Administrador de credenciales de Windows.
- **Configuración → Memoria**: recuerdos locales y sincronizados, cambios pendientes, rechazados y conflictos. Un conflicto muestra las dos versiones; eliges una o guardas una combinada.
- **Modo local** desactiva la sincronización: nada sale del equipo. Los cambios hechos mientras tanto se envían al volver al modo híbrido.
- En una conversación, si hay conexión, Jarvis completa lo que recuerda con la búsqueda del Cloud; esos recuerdos aparecen en el Neural Core en violeta y se traen al equipo en la siguiente sincronización.
- Las herramientas de Windows nunca se exponen al Cloud: el escritorio solo hace conexiones salientes.

Despliegue del Cloud Core: `../api/docs/06-infrastructure/contabo-deployment.md`.

## Pruebas

```powershell
powershell -ExecutionPolicy Bypass -File scripts\test.ps1
```

| Suite | Comando | Qué cubre |
|---|---|---|
| Rust | `cargo test` (en `apps/desktop/src-tauri`) | Ollama (servidor simulado), agente, política, registro, memoria (grafo, recuperación, exportación/importación), Tool API, cancelación |
| Interfaz | `npm test -w apps/desktop` | Eventos del agente, aprobación en la UI, reconexión de voz, eventos visuales, Neural Core, pantalla Memoria |
| Voz | `pytest` (en `services/voice-service`) | Máquina de estados, VAD, flujo completo, API y WebSocket, TTS, consentimiento |

Pruebas contra servicios reales (opcionales): `$env:JARVIS_TEST_OLLAMA='1'` y `$env:JARVIS_TEST_WHISPER='1'`. Escritorio ↔ Cloud Core con servicios reales (Linux): `../tests/desktop-cloud/run-local.sh` (escenarios A–G).

## Estructura

```text
assistant/
├── apps/desktop/            Tauri v2
│   ├── src/                 React 19 + Zustand + Tailwind v4
│   │   ├── app/  components/  features/{dashboard,assistant,voice,tasks,memory,neural,skills,activity,settings}
│   │   ├── services/ (backend.ts, voiceClient.ts, voiceApi.ts)   stores/
│   └── src-tauri/src/
│       ├── commands/        Superficie invocable desde la UI
│       ├── core/            agent · planner · executor · verifier · policies
│       ├── domain/          Tipos sin dependencias externas
│       ├── memory/          Neural Memory Engine: grafo, recuperación, embeddings, exportación/importación
│       ├── infrastructure/  llm (Ollama) · windows (Tool API) · persistence (SQLite y migraciones)
│       └── tools/           ToolRegistry, herramientas locales y proxy remoto
├── services/voice-service/  FastAPI: audio, VAD, wake word, STT, TTS, perfiles de voz
├── packages/
│   ├── shared-types/        Tipos TS de los contratos
│   ├── agent-protocol/      JSON Schema de eventos del agente y del WebSocket de voz
│   └── tool-schemas/        Esquemas de las herramientas locales (Rust los incrusta)
├── config/                  jarvis.toml · personality.yaml · voice.example.json
├── scripts/                 PowerShell: check-env, setup, dev, test, build, instaladores opcionales
└── docs/                    Arquitectura, voz, licencias, decisiones
```

## Datos locales

Todo vive en `%LOCALAPPDATA%\Jarvis\` (el windows-agent usa la subcarpeta `Agent\`):

| Archivo | Contenido |
|---|---|
| `jarvis.db` | Conversaciones, memorias y su grafo, experiencias, habilidades, tareas y auditoría |
| `backups\` | Copias automáticas antes de cada importación |
| `settings.json` | Cambios hechos en Configuración |
| `voice.json` | Configuración del servicio de voz |
| `voice-token` | Token local entre la app y el servicio de voz |
| `voices\` | Perfiles de voz clonada (con su registro de consentimiento) |
| `tts-cache\` | Caché de audios sintetizados |

Más detalle en [docs/architecture.md](docs/architecture.md).
