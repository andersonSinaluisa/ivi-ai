# jarvis-windows-agent

Proceso residente en Windows que recibe **tareas autorizadas** desde [Jarvis Cloud](../api/README.md) y las ejecuta de forma **segura, controlada, auditable, idempotente y verificable** mediante un sistema de **Tools** con permisos locales.

> Estado: **fases P1–P10 implementadas** (MVP, tools post-MVP, firmas y aprobación remota, updater, UI Automation y
> Local Tool API; estado por historia en [implementation-status.md](docs/10-backlog/implementation-status.md)).
> La carpeta [docs/](docs/README.md) sigue siendo la fuente de verdad del diseño; el protocolo de transporte lo define
> Jarvis Cloud ([api/docs/03-protocols/mqtt-protocol.md](../api/docs/03-protocols/mqtt-protocol.md)).

## Estructura

```text
crates/
  jarvis-protocol/  Protocolo MQTT v1 del Cloud (envelope, topics, mensajes) + contract tests
  jarvis-tooling/   Contrato de Tool, ToolResult, registro, locks, executor y verificación
  jarvis-policy/    policy.toml: principals, capabilities, riesgo, catálogo de apps, decisiones
  jarvis-state/     SQLite: inbox/dedup, ejecuciones, aprobaciones, outbox, recuperación
  jarvis-windows/   Único crate con `unsafe`: Win32/COM, Credential Manager, named pipe, sesión
  jarvis-agent/     Pipeline de tareas, Cloud Link, tools del MVP, CLI y canal local
```

Los cuatro primeros crates no dependen de Windows y se prueban con dobles; `jarvis-agent` los compone.

## Compilar y probar

```bash
cargo build --workspace
cargo test --workspace      # 250 pruebas: unitarias, contrato con el Cloud, estado, Win32 real, seguridad, fuzzing y pipeline con dobles
cargo clippy --workspace --all-targets
cargo fmt --all -- --check

# Aceptación contra el Cloud real: WIN-E2E-01…14 con evidencia archivada (última: 14 PASS, 0 FAIL)
pwsh -File scripts/e2e.ps1 -Email owner@example.com -Password '<passphrase>'
```

## Uso

```bash
# 1. Provisionar (el secreto se pide por consola; nunca como argumento)
jarvis-agent provision --device-id <uuid> --host jarvis.example.com --owner-user-id <uuid del owner>

# 2. Ejecutar (en desarrollo, con logs por consola)
jarvis-agent run --console

# 3. Operar desde la misma PC
jarvis-agent status
jarvis-agent tools list
jarvis-agent policy check --principal cloud:user:<uuid> --tool close_app --input '{"appId":"notepad","force":true}'
jarvis-agent approvals list
jarvis-agent approvals approve <taskId>
jarvis-agent invoke --tool is_app_running --input '{"appId":"chrome"}'   # misma política y verificación, sin Cloud

# 4. Instalar para que arranque al iniciar sesión (sin administrador, sin elevación)
jarvis-agent install --start
jarvis-agent diagnostics          # paquete de soporte redactado, para compartir con ayuda
jarvis-agent uninstall [--purge]  # quita tarea, credencial y binario (--purge borra también los datos)

# 5. Operación continua
jarvis-agent reconnect            # fuerza reconexión (tras reprovisionar o si cambió la red)
jarvis-agent policy reload        # relee policy.toml sin reiniciar
jarvis-agent update --check       # ¿hay versión nueva firmada? (--apply la deja activa, con rollback)
```

Contra el entorno de desarrollo del Cloud (Mosquitto local sin TLS) se añade `--plaintext`, permitido
solo para hosts loopback ([ADR-016](docs/09-decisions/ADR-016-mqtt-transport-and-loopback.md)).
`--root <ruta>` usa otra carpeta de datos en lugar de `%LOCALAPPDATA%\Jarvis\Agent`.

## Principios

1. **El Cloud solicita, el agente decide.** Un comando remoto nunca es un ejecutable confiable: pasa por validación, Permission Engine, riesgo y (si aplica) aprobación **locales**. Aunque el Cloud esté comprometido, el daño posible está acotado por la política local.
2. **Ejecutar no es completar.** *Jarvis no considera una tarea completada porque la ejecución no produjo una excepción. Una acción se considera completada únicamente cuando el resultado observable satisface la condición de éxito definida por la Tool.*
3. **Sin ejecución arbitraria.** No existe `cmd.exe /c <input remoto>`. Solo tools declaradas, apps de un catálogo y comandos con nombre definidos localmente.
4. **Sin IA en este proyecto.** El agente funciona sin LLM. El futuro `jarvis-agent-core` usa las mismas tools a través de la [Local Tool API](docs/08-future/agent-core-integration.md) (named pipe, JSON-RPC), sin duplicarlas.
5. **Mínimo privilegio.** Usuario estándar, sesión interactiva, sin UAC desactivado ni credenciales administrativas.

## Stack (resumen)

Rust + Tokio · `windows-rs` (Win32, COM, UI Automation, WinHTTP y WinVerifyTrust) · `rumqttc` (MQTT 3.1.1 sobre TLS con `rustls` y el almacén de certificados de Windows, [ADR-016](docs/09-decisions/ADR-016-mqtt-transport-and-loopback.md)) · `serde` · `tracing` · SQLite embebido (`rusqlite`, bundled) · Windows Credential Manager. Justificación en [ADR-001](docs/09-decisions/ADR-001-rust-agent-runtime.md).

## Cómo leer la documentación

1. [Visión](docs/00-overview/vision.md) → [Alcance](docs/00-overview/scope.md) → [MVP](docs/00-overview/mvp.md) → [Compatibilidad con el Cloud](docs/00-overview/cloud-compatibility.md)
2. [Arquitectura](docs/01-architecture/windows-agent-architecture.md) → [Pipeline de ejecución](docs/01-architecture/execution-pipeline.md)
3. [Sistema de tools](docs/03-tools/tool-system.md) → [Contrato](docs/03-tools/tool-contract.md) → [Verificación](docs/03-tools/verification.md)
4. [Seguridad](docs/04-security/security-architecture.md) → [Permisos](docs/04-security/permission-engine.md) → [Riesgo](docs/04-security/risk-model.md)
5. [Backlog](docs/10-backlog/user-stories.md) y [orden de implementación](docs/10-backlog/implementation-order.md)

Índice completo: [docs/README.md](docs/README.md).

## Cómo pedir implementación

```text
Implementa HU-WIN-014 utilizando docs/ como fuente de verdad.
```

Las historias están en [docs/10-backlog/stories/](docs/10-backlog/stories/). La primera es **HU-WIN-001**.
