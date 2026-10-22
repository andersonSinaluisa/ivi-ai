# Command Execution (P7, post-MVP)

## Principio

`run_command` **no es** ejecución remota arbitraria. El Cloud no envía líneas de comandos: envía un `commandId` de un catálogo definido **localmente** por el owner, más parámetros tipados que se validan contra el esquema del comando. Nunca se construye `cmd.exe /c <input remoto>` ni `powershell -Command <input remoto>`. Ver [ADR-008](../09-decisions/ADR-008-restricted-command-execution.md).

## Por qué

| Alternativa | Problema |
|---|---|
| Shell libre (`cmd /c`, `powershell -c`) | Un Cloud o una cuenta comprometida = RCE total del PC del usuario. Inyección trivial con `&`, `\|`, `;`, `$()` |
| Lista blanca de ejecutables con args libres | `powershell.exe -EncodedCommand …`, `rundll32`, `mshta`, `certutil -urlcache` (LOLBins) → igual a shell libre |
| **Comandos nombrados con plantilla** | El conjunto de efectos posibles es finito y revisado por el owner |

## Enfoques analizados

| Enfoque | Ejemplo | Seguridad | Flexibilidad | Decisión |
|---|---|---|---|---|
| **Named Commands** | `git.status`, `dotnet.build`, `dotnet.test`, `npm.test` sin parámetros | Máxima: efecto fijo y revisado | Baja | ✅ Base del diseño |
| **Command Templates** | `dotnet build {project}` con `project` validado (enum o patrón anclado, dentro de allowed roots) | Alta si cada parámetro está tipado | Media | ✅ Un named command **es** una plantilla con cero o más parámetros tipados |
| **Restricted Shell** | Intérprete propio o PowerShell Constrained Language Mode / JEA con lista de cmdlets | Difícil de garantizar: los escapes de lenguaje son históricos (CLM bypasses), JEA requiere admin y WinRM | Alta | ❌ No en MVP ni P7 |

Por eso P7 implementa **named commands con plantillas tipadas**: los IDs usan notación con punto (`dotnet.test`), que también es el nombre del lock `COMMAND:<id>` y de la capability `commands.run:<id>`.

## Evolución hacia el Agent Core

Cuando `jarvis-agent-core` necesite una terminal más flexible (p. ej. un agente de programación), el camino previsto **no** es relajar `run_command`:

1. **Más plantillas**: la mayoría de las necesidades de un agente de código son finitas (`git.diff`, `cargo.test`, `npm.run {script ∈ package.json}`). Los parámetros se validan contra el contexto (el script existe en `package.json`, el proyecto está en una raíz permitida).
2. **Workspace sandbox**: una tool separada `workspace.exec` (R3, siempre con aprobación o con política por workspace) que ejecuta dentro de un contenedor/AppContainer o Windows Sandbox con solo el directorio del repositorio montado, sin credenciales del usuario y sin red salvo lista. El daño posible queda en el sandbox, no en el perfil del usuario.
3. **Solo local**: esa capacidad se concede a `local:agent-core`, nunca a principals `cloud:*`.

Esto mantiene la regla: ningún texto recibido por MQTT se interpreta como línea de comandos.

## Catálogo local

```toml
# policy.toml
[commands."network.flush_dns"]
description = "Vaciar la caché DNS"
executable = "%SystemRoot%\\System32\\ipconfig.exe"
args = ["/flushdns"]
risk = "R2"
timeout_ms = 10000
expected_exit_codes = [0]
capture_output = false

[commands."network.ping"]
description = "Ping a un host de la lista"
executable = "%SystemRoot%\\System32\\PING.EXE"
args = ["-n", "4", "{host}"]
params.host = { type = "enum", values = ["1.1.1.1", "8.8.8.8", "router.local"] }
risk = "R1"
timeout_ms = 15000
expected_exit_codes = [0, 1]
capture_output = true
max_output_bytes = 8192

[commands."notes.backup"]
description = "Ejecutar el script de respaldo de notas"
executable = "%USERPROFILE%\\Tools\\backup-notes.exe"
args = ["--target", "{target}"]
params.target = { type = "string", pattern = "^[A-Za-z0-9_-]{1,32}$" }
risk = "R3"
timeout_ms = 120000
expected_exit_codes = [0]
```

## Reglas de ejecución

1. `commandId` inexistente → `COMMAND_NOT_FOUND` (rechazo `INVALID_PAYLOAD`).
2. Parámetros validados por tipo: `enum`, `string` con `pattern` obligatorio y anclado, `int` con rango. Sin parámetros libres sin patrón.
3. `executable` es ruta **absoluta**; se verifica que exista y (P8) su firma Authenticode si `require_signed = true`. No se resuelve por `PATH`.
4. Se prohíbe como `executable`: `cmd.exe`, `powershell.exe`, `pwsh.exe`, `wscript.exe`, `cscript.exe`, `mshta.exe`, `rundll32.exe`, `regsvr32.exe`, `bash.exe`, `wsl.exe` (validación al cargar la política). Un script legítimo se empaqueta como ejecutable del owner o se permite explícitamente como excepción futura con revisión.
5. Lanzamiento con `CreateProcessW` y `lpApplicationName` explícito, línea de comandos construida con **escapado de argumentos de Windows** (reglas de `CommandLineToArgvW`) por cada argumento sustituido.
6. Sin herencia de handles salvo stdout/stderr por pipe; entorno mínimo (sin variables con secretos); directorio de trabajo fijo configurable.

> **Implementado**: el proceso nace con `CREATE_SUSPENDED | CREATE_NO_WINDOW`, se asigna al Job Object y solo entonces se reanudan sus hilos, de modo que no puede crear descendencia fuera del job ni siquiera durante el instante inicial. La construcción de la línea de comandos usa el escapado de `CommandLineToArgvW` de la biblioteca estándar de Rust (`lpApplicationName` explícito, sin `PATH`, sin shell), y del entorno heredado se quitan las variables `JARVIS*`.

7. Proceso dentro de un **Job Object** con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, límite de memoria y de procesos activos. Cancelación/timeout = `TerminateJobObject` (mata también los hijos).
8. Salida: capturada solo si `capture_output = true`, truncada a `max_output_bytes` (≤ 24 KiB), decodificada como UTF-8 con reemplazo. **Nunca** se escribe en logs; se redacta en BD tras *settled*.
9. Integridad media (usuario estándar). Un comando nunca se eleva; si requiere admin, falla (`ACCESS_DENIED`).

## `run_command`

| Aspecto | Valor |
|---|---|
| Permiso | `commands.run` **y** `commands.run:<commandId>` |
| Riesgo | El declarado por el comando (R1–R3); R4 no permitido para comandos |
| Input | `{ "commandId": string, "params": object }` |
| Output | `{ "commandId", "exitCode", "stdout"?, "stderr"?, "truncated" }` |
| Recursos | `COMMAND:<commandId>` (exclusivo: no dos instancias simultáneas del mismo comando) |
| Timeouts | `timeout_ms` del comando, tope global 10 min |
| Cancelación | `kill` (Job Object). El bucle de espera mira una bandera de cancelación y llama a `TerminateJobObject`, así que un `task.cancel` detiene de verdad al hijo y a sus procesos; el resultado distingue cancelación de timeout |
| Concurrencia | Máximo 2 comandos simultáneos en todo el agente, además del lock por `commandId` |
| Verificación | `exit_code_expected`: `exitCode ∈ expected_exit_codes`. Opcionalmente `verify` adicional por comando (p. ej. existe un archivo de salida) |
| Sensibilidad | `output` |

## Riesgo residual

El owner puede definir un comando peligroso. Es su decisión local y consciente; el agente la limita con la lista de ejecutables prohibidos, el riesgo declarado (R3 → aprobación) y auditoría.
