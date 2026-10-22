# Active Window

## `get_active_window`

Salida estructurada:

```json
{
  "appId": "vscode",
  "process": "Code.exe",
  "pid": 1234,
  "title": "Jarvis - Visual Studio Code",
  "titleRedacted": false,
  "sessionState": "Active"
}
```

Implementación: `GetForegroundWindow` → `GetWindowThreadProcessId` → nombre de imagen (`QueryFullProcessImageNameW`, solo el nombre de archivo) → `appId` si coincide con el catálogo → `GetWindowTextW` solo si se pide y se permite el título.

| Caso | Resultado |
|---|---|
| Sesión bloqueada / Secure Desktop (UAC) | `GetForegroundWindow` devuelve `NULL` o una ventana inaccesible → `{ "sessionState": "Locked" }` sin ventana |
| Ventana de proceso elevado | Se devuelve `process` y `pid`; el título puede no ser legible (UIPI) → `titleRedacted: true` |
| Escritorio vacío | `process: "explorer.exe"`, sin título |

## Privacidad del título

Los títulos de ventana **contienen datos personales con frecuencia**:

| App | Título típico |
|---|---|
| Navegador | Título de la página: banca, salud, búsquedas, correo (`Inbox (3) - persona@dominio.com`) |
| Cliente de correo/chat | Asunto del mensaje, nombre del contacto |
| Editor | Ruta y nombre de archivos (`contrato-despido-juan.docx`) |
| Gestor de contraseñas | Nombre de la entrada o del sitio |

### Política

| Regla | Detalle |
|---|---|
| Por defecto sin título | `includeTitle = false` por defecto; con `true` requiere la capability `windows.window.title` |
| Apps con título siempre oculto | Lista local `[privacy] never_title = ["KeePassXC.exe", "1Password.exe", "Bitwarden.exe", "mstsc.exe"]` y navegadores en modo privado (clase o sufijo "InPrivate"/"Incognito") → `titleRedacted: true` |
| Modo de título | `[privacy] title_mode = "full" \| "app_only" \| "redact_patterns"` (default `redact_patterns`) |
| Redacción por patrones | Emails, números largos (tarjetas, cuentas, teléfonos), URLs con query → `[REDACTED]` |
| Longitud | Máx. 256 caracteres |
| Logs | El título **nunca** se loguea; solo `titleLength` y `titleRedacted` |
| Almacenamiento | El `ToolResult` con título se redacta en SQLite tras *settled* |

### Trade-off

Un título completo es muy útil para el futuro Agent Core ("¿qué documento tengo abierto?"). Se prioriza la privacidad por defecto; el owner lo habilita explícitamente para principals concretos (en la práctica, `local:agent-core`, que no envía el título fuera de la PC salvo al LLM, algo que debe decidir el Agent Core con su propia política).
