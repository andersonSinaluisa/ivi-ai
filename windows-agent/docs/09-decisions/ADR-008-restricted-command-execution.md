# ADR-008: Ejecución de comandos restringida

## Status

Accepted, 2026-09-19 (implementación en P7).

## Context

Ejecutar comandos es útil (`git status`, `dotnet test`), pero `MQTT → cmd.exe /c <input remoto>` convierte cualquier compromiso del Cloud o de una cuenta en ejecución remota de código en la PC del usuario.

## Decision

- No existe tool de shell ni capability `shell.execute`.
- `run_command` solo acepta un `commandId` de un **catálogo local** de named commands/plantillas con parámetros tipados (enum, patrón anclado, int con rango).
- Ejecutable absoluto, `CreateProcessW` sin intérprete, escapado por argumento, intérpretes prohibidos como ejecutable, Job Object con límites, salida truncada y nunca logueada.
- Riesgo declarado por comando; capability `commands.run:<id>` por comando.
- `open_app` no acepta argumentos ni URLs.

Detalle en [Command Execution](../03-tools/command-execution.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Shell libre | RCE |
| Lista blanca de ejecutables con argumentos libres | LOLBins (`powershell -EncodedCommand`, `rundll32`, `mshta`) equivalen a shell |
| Restricted shell (PowerShell CLM/JEA) | Historial de bypasses; JEA requiere admin y WinRM |

## Consequences

- ✅ Conjunto finito y revisado de efectos posibles.
- ⚠️ Menos flexible: cada comando nuevo lo añade el owner localmente.
- ⚠️ El futuro Agent Core de programación necesitará un sandbox de workspace separado (documentado como evolución, no como relajación de esta regla).
