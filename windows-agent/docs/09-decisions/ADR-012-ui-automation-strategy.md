# ADR-012: UI Automation como mecanismo de UI

## Status

Accepted, 2026-09-19. Implementación en P9.

## Context

Tras el MVP, Jarvis necesitará interactuar con el contenido de apps (pulsar botones, leer campos). Las opciones van desde UIA hasta coordenadas de pantalla.

## Decision

- **Windows UI Automation** (`IUIAutomation` vía `windows-rs`) es el mecanismo para UI de apps de escritorio.
- Selectores por `AutomationId` → `ControlType` + `Name` → `ClassName` → ruta estructural; nunca coordenadas. Selector ambiguo = error.
- Interacción mediante patrones (`Invoke`, `Value`, `Toggle`, `SelectionItem`, `ExpandCollapse`), no con clics simulados.
- Hilo STA dedicado; los objetos COM no salen de él; `CacheRequest`; timeouts por llamada y recreación del hilo si se bloquea.
- Tools que actúan toman `DESKTOP_INPUT`; verificación por estado del elemento.

Detalle en [UI Automation](../05-windows/ui-automation.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| MSAA (`IAccessible`) | Legado, menos información; UIA lo cubre vía proxies |
| Coordenadas + `SendInput` | Frágil, no verificable, interfiere con el usuario |
| WinAppDriver / Appium | Servicio externo con puerto HTTP local, proyecto poco mantenido |
| `uiAccess=true` para apps elevadas | Requiere firma e instalación en Program Files; amplía privilegios |

## Consequences

- ✅ Acciones semánticas, verificables y estables entre resoluciones.
- ⚠️ Apps sin accesibilidad no son automatizables por esta vía (fallback P11/P12).
- ⚠️ No funciona con sesión bloqueada ni con apps elevadas.
