# Privilege Model

## Reglas

1. El agente se ejecuta como el **usuario interactivo, estándar, con integridad media**.
2. **No** se ejecuta como Administrator de forma permanente, ni como `LocalSystem`, ni con `requireAdministrator` en el manifiesto (`asInvoker`).
3. **No** desactiva UAC ni modifica políticas de seguridad del sistema.
4. **No** almacena credenciales administrativas.
5. **No** se auto-eleva. Ninguna tool invoca `runas` ni usa `ShellExecuteExW` con verbo `runas`.
6. La instalación es **por usuario** (`%LOCALAPPDATA%`), sin necesidad de admin. Ver [ADR-011](../09-decisions/ADR-011-privilege-model.md).

## Por qué

| Si corriera como admin/SYSTEM | Consecuencia |
|---|---|
| Un bug de validación en una tool | Escritura en `System32`, instalación de drivers, desactivar Defender |
| Un Cloud comprometido | Control total de la máquina en lugar de "lo que el usuario puede hacer" |
| Como servicio en Session 0 | No ve el escritorio del usuario: no puede observar ventanas ni lanzar apps visibles ([Startup Model](../05-windows/startup-model.md)) |

Con integridad media, Windows aplica por nosotros:
- **UIPI**: no se pueden enviar mensajes ni input a ventanas de mayor integridad (apps elevadas, prompts de UAC).
- **ACLs NTFS**: no se escribe en `Program Files`, `Windows`, perfiles ajenos.
- **Tokens**: no se pueden abrir con `PROCESS_TERMINATE` procesos de otros usuarios o elevados.

## Qué ocurre con tareas que necesitarían admin

| Caso | Comportamiento |
|---|---|
| `close_app` sobre una app elevada | `APP_ELEVATED` (no se puede, y se informa claramente) |
| `open_app` de una app con `requireAdministrator` | Windows mostraría UAC en el Secure Desktop, que el agente no puede aceptar. Se detecta por `ERROR_ELEVATION_REQUIRED` y falla con `ACCESS_DENIED`; **no** se reintenta con `runas` |
| `run_command` que requiere admin | Falla (`ACCESS_DENIED`); el owner decide ejecutarlo manualmente |
| Cambios de configuración del sistema | Fuera de alcance (R4) |

## Futuro: operaciones privilegiadas acotadas

Si alguna vez se necesita una operación privilegiada concreta (p. ej. reiniciar un servicio concreto), el diseño aceptable es un **broker privilegiado mínimo**, instalado explícitamente por el admin, con una API cerrada (lista fija de operaciones con parámetros tipados), autenticación del llamante por SID y auditoría. Nunca un agente que corra entero con privilegios. No está planificado en el roadmap actual.

## Riesgo residual

Un usuario que sea administrador del equipo y tenga UAC desactivado ejecutaría el agente con token completo. El agente lo detecta al arrancar (`TokenElevation`) y:
- health `RUNNING_ELEVATED` (`WARN`),
- rechaza iniciar salvo `--allow-elevated` explícito (para pruebas),
- lo muestra en `jarvis-agent status`.
