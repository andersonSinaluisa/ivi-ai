# Estado de implementación

Estado real del código frente al [backlog](user-stories.md). Fecha: 2026-09-19.

Leyenda: ✅ implementada · 🟡 parcial (lo que falta se indica) · ⬜ no empezada.

## Resumen

| Fase | Épicas | Estado |
|---|---|---|
| P1 Foundation | EPIC-01 | ✅ |
| P2 Identity + Cloud | EPIC-02, EPIC-03 | ✅ |
| P3 State + Task intake | EPIC-04, HU-WIN-038 | ✅ |
| P4 Tool System + Permissions | EPIC-05, EPIC-06, HU-WIN-039…041 | ✅ |
| P5 Process Tools + Verification | EPIC-07, EPIC-08, HU-WIN-042 | ✅ |
| P6 Resilience + Packaging | EPIC-10, EPIC-11, EPIC-12 | 🟡 todo salvo la firma Authenticode del release, que necesita un certificado |
| P7 Post-MVP Tools | EPIC-13 | ✅ |
| P8 Firmas, aprobaciones remotas y updates | EPIC-14 | 🟡 el updater no se ha probado con un release firmado real (mismo bloqueo que HU-WIN-051) |
| P9 UI Automation | EPIC-15 | ✅ |
| P10 Local Tool API | EPIC-16 | ✅ |
| P11–P12 Browser y visión | EPIC-17, EPIC-18 | ⬜ fuera del alcance de este proyecto |

Pruebas: **252** (unitarias, contrato con el Cloud, Win32 real, pipeline con dobles, seguridad, fuzzing y vectores de firma compartidos con el Cloud) más los **16 escenarios de aceptación** contra el Cloud real ([`scripts/e2e.ps1`](../../scripts/e2e.ps1)).

Además del agente, el workspace incluye **`jarvis-sign`**: el firmador del lado del owner (genera el par de claves, firma tareas y decisiones de aprobación). No es parte del agente —la clave privada es del owner— y existe hasta que la Owner Console ocupe su lugar.

## Detalle por historia

| HU | Título | Estado | Notas |
|---|---|---|---|
| 001 | Workspace, crates y CI | ✅ | Workspace, lints (`unsafe_code` prohibido salvo `jarvis-windows`), `rust-toolchain.toml`, workflow `ci.yml` (fmt, clippy, tests en Linux y Windows, `cargo deny`) y comprobación de que la capa de decisión no depende de `windows` |
| 002 | Configuración `config.toml` | ✅ | Validación estricta y rechazo de claves con pinta de secreto |
| 003 | Logging estructurado | 🟡 | JSON con rotación diaria, niveles, spans por tarea, poda por retención y tipo `Sensitive`; **falta** la agregación de mensajes repetidos |
| 004 | Ciclo de vida del proceso | ✅ | Instancia única por SID, rechazo de arranque elevado, apagado por Ctrl+C, por `stop` y por `WM_QUERYENDSESSION` (ventana *message-only*) |
| 005 | CLI y named pipe | ✅ | ACL restringida al SID del usuario: `status`, `stop`, `reconnect`, `approvals`, `policy`, `tools`, `invoke`, `diagnostics` |
| 006 | Crate `jarvis-protocol` | ✅ | Contract tests que leen los ejemplos de la documentación del Cloud |
| 007 | Conexión MQTT/TLS | ✅ | MQTT 3.1.1, `rustls` con verificador de plataforma, LWT, clean start ([ADR-016](../09-decisions/ADR-016-mqtt-transport-and-loopback.md)) |
| 008 | Presencia | ✅ | hello/heartbeat/goodbye con `sessionState`; sin hostname ni IP |
| 009 | Reconexión y backoff | ✅ | Backoff con jitter, `AUTH_REJECTED` (5 min, interrumpible al reprovisionar), `CLOUD_UNREACHABLE` y reconexión inmediata al reanudar de suspensión |
| 010 | Validación de mensajes entrantes | ✅ | Tamaño, envelope, `deviceId`, topic propio y límite de 50 mensajes/s con aviso agregado |
| 011 | Estado de sesión | ✅ | Eventos `WM_WTSSESSION_CHANGE` más `GetLastInputInfo` para el ocioso |
| 012 | Credential Manager | ✅ | `CredWriteW`/`CredReadW`/`CredDeleteW`, `Secret` con `zeroize` |
| 013 | `provision` | ✅ | Secreto por consola o stdin (tolera el BOM que antepone PowerShell), prueba de conexión antes de guardar, política inicial |
| 014 | Credencial en el Cloud Link | ✅ | Lectura en cada conexión, `NOT_PROVISIONED`, recuperación tras rotar sin reiniciar el agente (WIN-E2E-04) |
| 015 | SQLite Store | ✅ | WAL, `synchronous=FULL`, escritor único, migraciones con copia previa |
| 016 | Dedup de tareas | ✅ | Reenvío del último estado; nunca se re-ejecuta (verificado publicando 100 veces el mismo dispatch, WIN-E2E-03) |
| 017 | Outbox y settle | ✅ | Persistencia previa a publicar, reenvío hasta `task.update.ack` |
| 018 | Recuperación de arranque | ✅ | `EXECUTION_INTERRUPTED`, re-encolado, expiración de aprobaciones y modo seguro tras 3 arranques sucios en 10 min |
| 019 | Retención y redacción | ✅ | Limpieza a 8 días y redacción del resultado sensible tras *settled* |
| 020 | Contrato de Tool y `ToolResult` | ✅ | `jarvis.tool-result/1` con ejecución y verificación separadas |
| 021 | Registro y mapeo TaskType | ✅ | Alias `OPEN_CHROME`, validación de descriptores al arrancar |
| 022 | Validación de input | 🟡 | Tipos estrictos + esquema publicado en el descriptor; la tool `noop` vive en los tests del executor |
| 023 | Executor | ✅ | Locks, concurrencia, timeout efectivo, cancelación, aislamiento de pánicos, `TOOL_STUCK` |
| 024 | Carga de `policy.toml` | ✅ | Modelo, validación, fail-closed y recarga en caliente (`policy reload`, vigilancia del archivo) |
| 025 | Evaluación de riesgo | ✅ | Riesgo efectivo con modificadores y umbrales |
| 026 | Permisos y `policy check` | ✅ | Algoritmo completo y explicación por CLI |
| 027 | Aprobaciones locales | ✅ | Estado durable, CLI, expiración, re-evaluación y aviso por globo del área de notificación |
| 028 | Plataforma de procesos y ventanas | ✅ | Filtrado por sesión y usuario, integridad, ventanas reales |
| 029 | Catálogo de aplicaciones | ✅ | `app_paths`, `aumid`, `path`, alias y ubicaciones de confianza |
| 030–034 | Tools del MVP | ✅ | `is_app_running`, `list_running_apps`, `get_active_window`, `open_app`, `close_app` |
| 035–037 | Verificación | ✅ | Motor con sondeo, métodos `process_and_window` y `process_absent`, reflejo en mensajes |
| 038–041 | Pipeline de tareas | ✅ | 17 etapas, cancelación y aprobaciones integradas |
| 042 | Vertical slice | ✅ | Demostrado contra el Cloud real |
| 043 | Corrupción de SQLite | ✅ | Detección, apartado de la base, `STATE_RESET`, `status --ack-reset` y copia previa a migrar |
| 044 | Suite de fallos | ✅ | Con dobles en `tests/pipeline.rs` y contra el broker real en WIN-E2E-09/10 (corte del broker y muerte del proceso a mitad de tarea) |
| 045 | Protección de recursos | 🟡 | Concurrencia, `TOOL_STUCK`, límite de tasa de entrada y Job Object de 512 MB sobre el propio proceso; **falta** la medición formal de consumo en reposo |
| 046 | Datos sensibles | ✅ | Canarios en tests, redacción en logs, BD y diagnóstico |
| 047 | Fuzzing del parser | 🟡 | Batería determinista de 40 000 entradas hostiles y mutadas (`jarvis-protocol/tests/fuzz_parsers.rs`); **falta** la campaña nocturna con `cargo-fuzz` en CI |
| 048 | Endurecimiento del proceso | ✅ | Mitigaciones de proceso, ACL del pipe, rechazo de elevación, sin puertos en escucha |
| 049 | Suite del threat model | ✅ | `tests/threat_model.rs`: una prueba por amenaza mitigable (WT-01…WT-15 aplicables) |
| 050 | `install` / `uninstall` | ✅ | Binario en `%LOCALAPPDATA%\Programs\Jarvis\Agent`, Scheduled Task al iniciar sesión sin elevación, `--start`, `--purge` |
| 051 | Build de release firmada | 🟡 | Workflow `release.yml` con build, SBOM y hashes; el paso de firma Authenticode existe pero **no está verificado**: requiere un certificado real, y sin él el artefacto sale sin firmar y el workflow avisa |
| 052 | `diagnostics` | ✅ | Carpeta con health, config, política, resumen de tareas y logs con segunda redacción |
| 053 | Aceptación del MVP | ✅ | [`scripts/e2e.ps1`](../../scripts/e2e.ps1) ejecuta WIN-E2E-01…14 contra el Cloud real: **14 PASS, 0 FAIL** el 2026-09-19, con evidencia en `scripts/e2e-evidence/20260919-213733/` |
| 054 | Sandbox de rutas | ✅ | Canonicalización, raíces permitidas, rechazo de enlaces y namespaces |
| 055–056 | `list_directory`, `read_file` | ✅ | Con límites de tamaño, conteo y tipos |
| 057 | Comandos con nombre | ✅ | Plantillas tipadas, sin shell, Job Object, salida acotada y cancelación real del árbol de procesos |
| 058 | `run_command` | ✅ | R3 salvo lo que la política baje, con verificación por código de salida |
| 059–060 | `focus_window`, `show_notification` | ✅ | Lock `DESKTOP_INPUT` y notificación sin botones (WT-15) |
| 061 | Firma de tareas | ✅ | Ed25519, umbral por riesgo, firma sobre el mensaje canónico; una firma inválida se rechaza incluso por debajo del umbral. **Probado contra el Cloud real** desde que este emite firmas (WIN-E2E-15) |
| 062 | Aprobación remota firmada | ✅ | Claves de aprobador propias, ventana de frescura, nonce de un solo uso y decisión atada al payload (WIN-E2E-16) |
| 063 | Updater con rollback | 🟡 | Manifiesto firmado con clave offline, anti-downgrade, SHA-256, Authenticode, instalación por versiones y rollback automático; **no probado** con un release firmado real (depende de HU-WIN-051) |
| 064 | Host UIA en hilo STA | ✅ | Hilo dedicado con `CoInitializeEx(APARTMENTTHREADED)`, API por canal, DTOs planos y abandono del hilo si deja de responder |
| 065 | `find_window` / `find_element` | ✅ | Selectores semánticos; `ELEMENT_AMBIGUOUS` si hay más de uno, `ELEMENT_NOT_FOUND` si no hay ninguno |
| 066 | `invoke_element`, `set_text`, `get_text` | ✅ | Patrones UIA (nunca coordenadas), verificación por cambio observable y `ValuePattern.Value` |
| EPIC-16 | Local Tool API | ✅ | `tools/list` y `tools/call` en JSON-RPC 2.0 sobre named pipe, principal `local:agent-core`, presupuesto de llamadas y deduplicación por `invocationId` |

## Hallazgos de la ejecución contra el Cloud real

Los escenarios de aceptación destaparon cuatro fallos que no veían los tests con dobles. Todos corregidos y con prueba:

| Qué fallaba | Consecuencia | Arreglo |
|---|---|---|
| El CONNACK negativo de rumqttc llega como *error*, no como paquete | Una credencial revocada se veía como "sin red": nunca aparecía `AUTH_REJECTED` | Se clasifica `ConnectionRefused(NotAuthorized \| BadUserNamePassword)` como rechazo de autenticación |
| `Notify::notify_waiters()` para pedir reconexión | La petición se perdía si el enlace no estaba justo esperando | `notify_one()`, que guarda el permiso |
| El backoff de 5 min por `AUTH_REJECTED` no se podía interrumpir | Tras reprovisionar había que esperar o reiniciar el agente | Todas las esperas del enlace escuchan la petición de reconexión |
| `run_command` declaraba cancelación pero ejecutaba en un hilo bloqueante sin señal | `task.cancel` no detenía el proceso hijo: la cancelación era solo nominal | Bandera de cancelación hasta el bucle de espera, que mata el Job Object; `CommandOutput.cancelled` lo distingue del timeout |

Además, `provision --secret-stdin` ahora tolera el BOM que PowerShell antepone a `stdin` (sin eso, el secreto llegaba corrupto y el broker respondía `NotAuthorized` sin explicación).

## Desviaciones respecto al diseño

Cada una está reflejada ya en el documento correspondiente:

| Tema | Diseño original | Implementación | Dónde |
|---|---|---|---|
| Versión MQTT | v5 con fallback | 3.1.1 | [ADR-016](../09-decisions/ADR-016-mqtt-transport-and-loopback.md) |
| TLS | Nunca desactivable | Texto plano solo en loopback | [ADR-016](../09-decisions/ADR-016-mqtt-transport-and-loopback.md), [MQTT Client](../02-cloud-integration/mqtt-client.md) |
| `appId` | `[a-z0-9_-]` estricto | Id, nombre visible o alias del catálogo; nunca rutas | [Process Tools](../03-tools/process-tools.md) |
| Identificación de la app | Ruta de imagen exacta | Ruta exacta **o** ubicación de confianza | [Process Management](../05-windows/process-management.md#identificación-de-la-app-implementado), [WT-09](../04-security/threat-model.md#wt-09-malicious-application-impersonation) |
| `open_app.ifRunning` | `noop\|focus\|new_instance` | `noop\|new_instance` (`focus` con `focus_window` en P7) | [Process Tools](../03-tools/process-tools.md) |
| Guardia `STATE_RESET` | `createdAt < db_created_at` | Con margen de 60 s (reloj no monótono) | [Idempotency](../06-state/idempotency.md#reinicio-de-la-base-de-datos) |
| Validación de input | JSON Schema en runtime | Tipos estrictos; el esquema se publica en el descriptor | [Tool Contract](../03-tools/tool-contract.md) |
| Authenticode | Cualquier firma válida | Solo firma **incrustada**; las de catálogo (`.cat`) no cuentan | [Startup Model](../05-windows/startup-model.md#actualizaciones) |
| Local Tool API | Principals `local:agent-core` y `local:owner` | Siempre `local:agent-core`: por ese canal nadie puede atribuirse los permisos del owner | [Agent Core Integration](../08-future/agent-core-integration.md#seguridad-del-agent-core) |
| Dedup de la Local Tool API | Misma tabla de dedup que las tareas | En memoria del proceso (64 últimas invocaciones) | [Agent Core Integration](../08-future/agent-core-integration.md#estado-de-implementación) |
| Estado `Rejected` en el Cloud | El agente rechaza y el Cloud muestra `Rejected` | El Cloud no tiene ese estado: `task.rejected` aterriza como `Failed` con el código del agente | [Cloud Compatibility](../00-overview/cloud-compatibility.md) |
