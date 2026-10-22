# EPIC-01 — Agent Foundation

Fase **P1**. Objetivo: un proceso Rust estable, configurable, observable y controlable por CLI, sin red ni tools. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-001 — Workspace Cargo, crates y CI

**Goal**: disponer de la estructura de código y del pipeline de calidad sobre la que se construirán todas las HUs.

**Description**: crear el workspace Cargo con los crates definidos en [Windows Agent Architecture](../../01-architecture/windows-agent-architecture.md#estructura-de-crates-workspace-cargo): `jarvis-protocol`, `jarvis-tooling`, `jarvis-policy`, `jarvis-state`, `jarvis-windows`, `jarvis-agent` (bin). Fijar toolchain, lints y CI.

**Acceptance Criteria**
1. `cargo build --workspace` compila en Windows x64; los crates marcados como portables compilan también en Linux (`cargo build -p jarvis-protocol -p jarvis-tooling -p jarvis-policy -p jarvis-state`).
2. `rust-toolchain.toml` fija una versión estable; edición 2024.
3. Lints de workspace: `unsafe_code = "forbid"` en todos los crates salvo `jarvis-windows`; `clippy::pedantic` en warn con excepciones documentadas; `-D warnings` en CI.
4. CI (GitHub Actions) con jobs: `fmt --check`, `clippy`, `test` en `ubuntu-latest` (crates portables) y `windows-latest` (workspace completo), `cargo deny` (licencias y advisories).
5. `jarvis-agent --version` imprime `jarvis-windows-agent <semver> (<commit>)`.
6. README del código apunta a `docs/` como fuente de verdad.

**Dependencies**: ninguna.

**Technical Considerations**: `[workspace.dependencies]` para versiones únicas; `Cargo.lock` versionado; `windows-rs` solo como dependencia de `jarvis-windows`; `build.rs` para embeber commit.

**Tests**: un test trivial por crate para validar la ejecución en CI; job que falla si `jarvis-policy` depende de `windows` (`cargo tree -p jarvis-policy | grep windows` vacío).

**Definition of Done**: DoD común + pipeline verde en ambos sistemas operativos.

---

## HU-WIN-002 — Carga de configuración (`config.toml`)

**Goal**: configurar el agente sin recompilar y sin secretos en archivos.

**Description**: cargar y validar `%LOCALAPPDATA%\Jarvis\Agent\config.toml` según [Configuración local](../../06-state/local-state.md#configuración-local).

**Acceptance Criteria**
1. Secciones `device`, `cloud`, `mqtt`, `tools`, `executor`, `logging` con valores por defecto documentados.
2. Claves desconocidas → error con línea y columna.
3. Claves con nombre `password`, `secret`, `token`, `key` (en cualquier sección) → el agente se niega a arrancar con un mensaje que explica cómo usar `provision`.
4. Archivo ausente → defaults; `device.device_id` ausente → estado `NOT_PROVISIONED` (no es un crash).
5. Rutas resueltas con `SHGetKnownFolderPath(FOLDERID_LocalAppData)`, no con variables de entorno.
6. `--config <ruta>` permite otra ubicación (tests).

**Dependencies**: HU-WIN-001.

**Technical Considerations**: `serde` + `toml` con `deny_unknown_fields`; tipos fuertes (`Url`, `NonZeroU16`); el módulo de config vive en `jarvis-agent` (rutas) con el modelo en un crate portable.

**Tests**: unit (válido, clave desconocida, clave sospechosa, tipos erróneos, defaults); snapshot de mensajes de error.

**Definition of Done**: DoD común.

---

## HU-WIN-003 — Logging estructurado con redacción

**Goal**: logs JSON útiles que nunca contengan datos sensibles.

**Description**: implementar el logging de [Logging](../../07-observability/logging.md): `tracing` JSON, rotación, niveles, tipos `Secret<T>`/`Sensitive<T>` y capa de redacción.

**Acceptance Criteria**
1. Logs en `logs\agent-YYYY-MM-DD.jsonl`, rotación diaria y por tamaño, retención configurable.
2. Nivel configurable (`logging.level`) y `run --console` para salida legible.
3. `Secret<T>` y `Sensitive<T>` imprimen `***` en `Debug`/`Display`; no implementan `Serialize` salvo con redacción.
4. Capa de redacción que reemplaza el valor de campos llamados `password`, `secret`, `token`, `authorization`.
5. Agregación de mensajes repetidos (`repeated: N`) cada 60 s.
6. Pánicos capturados con hook: log `ERROR` con ubicación (stack solo a nivel `DEBUG`).

**Dependencies**: HU-WIN-001, HU-WIN-002.

**Technical Considerations**: `tracing-subscriber` con `fmt::json`, `tracing-appender` non-blocking (guardar el `WorkerGuard` hasta el apagado).

**Tests**: test canario: loguear estructuras con `Secret("CANARY-…")` y verificar que el canario no aparece en el archivo; test de rotación con reloj falso.

**Definition of Done**: DoD común.

---

## HU-WIN-004 — Ciclo de vida del proceso

**Goal**: un proceso residente con instancia única y apagado ordenado.

**Description**: comando `jarvis-agent run` con el ciclo de vida de [Startup Model](../../05-windows/startup-model.md#ciclo-de-vida-del-proceso) (sin Cloud ni BD todavía).

**Acceptance Criteria**
1. Mutex `Local\Jarvis.Agent.<SID>`: una segunda instancia termina con código 3 y log `WARN`.
2. Apagado ordenado ante Ctrl+C, `WM_QUERYENDSESSION`/`WM_ENDSESSION` (ventana *message-only*) y `stop` por pipe (HU-WIN-005): cancela tareas, espera hasta 5 s, flush de logs.
3. Detección de token elevado (`TokenElevation`): sin `--allow-elevated` → no arranca, mensaje claro.
4. Health snapshot en memoria con `agent.state` (`STARTING`/`RUNNING`/`STOPPING`) y lista de condiciones.
5. Binario del agente sin ventana de consola cuando lo lanza la Scheduled Task.

**Dependencies**: HU-WIN-003.

**Technical Considerations**: `tokio::signal::ctrl_c`, hilo dedicado para el bucle de mensajes Win32; `CancellationToken` raíz propagado a los componentes.

**Tests**: integración Windows: dos instancias; envío de `WM_QUERYENDSESSION` a la ventana message-only; arranque elevado (runner CI es admin → verifica el rechazo).

**Definition of Done**: DoD común.

---

## HU-WIN-005 — CLI y canal local (named pipe)

**Goal**: controlar el proceso residente de forma segura desde la consola del usuario.

**Description**: CLI con `clap` y servidor de named pipe `\\.\pipe\jarvis-agent-<SID>` según [Component Model](../../01-architecture/component-model.md#cli-local). En esta HU: `status`, `stop`, `version`.

**Acceptance Criteria**
1. Pipe creado con security descriptor que solo concede acceso al SID del usuario (y a nadie más, ni Administrators) y `PIPE_REJECT_REMOTE_CLIENTS`.
2. El servidor verifica el SID del cliente (`GetNamedPipeClientProcessId` + token) antes de atender.
3. Protocolo JSON por línea con versión; mensajes ≤ 64 KiB.
4. `status` muestra el snapshot con el formato de [Health](../../07-observability/health.md#salida-de-jarvis-agent-status); códigos de salida 0/1/2.
5. Sin proceso residente: `status` informa "agent not running" (código 2).

**Dependencies**: HU-WIN-004.

**Technical Considerations**: `tokio::net::windows::named_pipe`; construcción del SDDL `D:P(A;;GA;;;<SID>)`.

**Tests**: integración Windows: cliente con otro usuario (si el runner lo permite) o test del descriptor generado; `status` con y sin agente.

**Definition of Done**: DoD común.
