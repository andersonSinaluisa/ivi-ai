# Risk Model

## Niveles

| Nivel | Nombre | Definición | Ejemplos | Default remoto (principal con capability) |
|---|---|---|---|---|
| **R0** | Observe | Lectura de estado no sensible, sin efectos | `is_app_running`, `list_running_apps` | `APPROVED` |
| **R1** | Sensitive read | Lectura que puede exponer datos personales, o efecto trivial | `get_active_window`, `list_directory`, `read_file`, `show_notification` | `APPROVED` |
| **R2** | Reversible action | Efecto visible, fácil de deshacer, sin pérdida de datos | `open_app`, `close_app` (graceful), `focus_window` | `APPROVED` |
| **R3** | Impactful action | Puede causar pérdida de datos o efectos difíciles de revertir | `close_app force`, `run_command` de riesgo alto, escritura de archivos (futuro) | `REQUIRES_APPROVAL` |
| **R4** | Critical | Destructivo, irreversible o que afecta la seguridad del sistema | Borrar archivos, cambiar configuración de seguridad, instalar software | `DENIED` remoto **siempre** |

## Riesgo efectivo

```text
riesgoEfectivo = max(tool.riskLevel, modificadores(input), riesgoDelComando)
```

| Modificador | Efecto |
|---|---|
| `close_app.force = true` | → R3 |
| `run_command` | Riesgo declarado por el comando en el catálogo (R1–R3) |
| `open_app.ifRunning = "focus"` | Sin cambio (R2), añade lock `DESKTOP_INPUT` |
| `get_active_window.includeTitle = true` | Sin cambio (R1), exige además `windows.window.title` |
| Sesión `Locked` | Sin cambio; las tools con `requiresInteractiveSession` fallan con `SESSION_UNAVAILABLE` |

El riesgo **nunca** baja por el input: los modificadores solo suben.

## Umbrales configurables

```toml
# policy.toml
[risk]
auto_approve_max = "R2"          # ≤ este nivel: APPROVED si hay capability
approval_required_max = "R3"     # (auto_approve_max, approval_required_max]: REQUIRES_APPROVAL
# por encima: DENIED. R4 remoto nunca se aprueba (fijo en código)
```

- El owner puede **bajar** `auto_approve_max` (p. ej. a `R1` para que `open_app` requiera aprobación), también por principal.
- Puede subir `auto_approve_max` a `R3` solo para un principal concreto y con advertencia en `policy check`; nunca `R4`.
- `R4` remoto está fijo en `DENIED` por código, no por configuración ([ADR-004](../09-decisions/ADR-004-local-permission-authority.md)).

## Equivalencia con el Cloud

El [Windows Agent Contract del Cloud](../../../api/docs/07-integrations/windows-agent-contract.md) usa una escala textual ([C14](../00-overview/cloud-compatibility.md)):

| Cloud | Agente |
|---|---|
| Bajo | R0–R1 |
| Medio | R2 |
| Alto | R3 |
| Prohibido | R4 |

## Por qué cinco niveles

- Separar **R0 de R1** permite conceder observación inocua (¿está Chrome abierto?) a un ESP32 sin concederle leer títulos de ventana o archivos.
- Separar **R2 de R3** es la frontera de "¿puede perderse trabajo del usuario?": ahí entra el humano.
- **R4** existe para que ciertas capacidades futuras no se puedan habilitar remotamente por un error de configuración.

## Trade-offs

| Decisión | Coste | Beneficio |
|---|---|---|
| R2 autoaprobado por defecto | Un Cloud comprometido puede abrir/cerrar apps del catálogo (sin `force`) | El caso principal (abrir Chrome con un botón) funciona sin fricción |
| R3 con aprobación **local** en MVP | Hasta P8 no hay aprobación remota: hay que estar frente a la PC | Nadie pierde datos por una orden remota no confirmada |
| Riesgo estático por tool, no aprendido | Menos flexible | Predecible, auditable y testeable |
