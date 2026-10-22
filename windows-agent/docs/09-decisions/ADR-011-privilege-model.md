# ADR-011: Usuario estándar, sin elevación

## Status

Accepted, 2026-09-19.

## Context

Correr como administrador simplificaría algunas tareas (cerrar apps elevadas, comandos del sistema), pero multiplicaría el impacto de cualquier bug o compromiso remoto.

## Decision

- El agente corre como el usuario interactivo, integridad media, manifiesto `asInvoker`.
- Nunca Administrator permanente, nunca `LocalSystem`, nunca `runas`, no desactiva UAC, no almacena credenciales administrativas.
- Instalación por usuario en `%LOCALAPPDATA%`.
- Si detecta token elevado, no arranca salvo `--allow-elevated` (pruebas) y reporta `RUNNING_ELEVATED`.
- Operaciones que requieran admin fallan con `ACCESS_DENIED` / `APP_ELEVATED`. Una futura necesidad privilegiada se resolvería con un broker mínimo instalado por el admin, con API cerrada.

Detalle en [Privilege Model](../04-security/privilege-model.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Agente como admin | Impacto total ante bugs o compromisos |
| Elevación bajo demanda (UAC) | El prompt aparece en el Secure Desktop, que un agente remoto no puede ni debe aceptar; fomenta aceptar prompts sin leer |
| Servicio `LocalSystem` + agente de sesión | Componente privilegiado permanente; fuera de alcance |

## Consequences

- ✅ Windows (UIPI, ACLs, integridad) actúa como última capa de defensa.
- ⚠️ No se controlan apps elevadas ni configuración del sistema; es explícito en los errores.
