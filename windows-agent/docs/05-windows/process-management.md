# Process Management

Cómo se implementan en Windows las [Process Tools](../03-tools/process-tools.md).

## Identificar "una app"

Una app no es un proceso. Casos reales:

| Caso | Ejemplo | Consecuencia |
|---|---|---|
| Múltiples procesos | Chrome, Edge, VS Code (Electron), Teams | "Está corriendo" = al menos un proceso; "cerrada" = ninguno |
| Launcher que termina | `Spotify.exe` o apps con updater que relanzan otro proceso | El PID lanzado muere y otro toma su lugar; la verificación no puede depender solo del PID |
| Instancia única | Segundo `chrome.exe` pasa la orden al existente y sale con código 0 | `open_app` con app ya abierta: el PID lanzado sale; éxito si hay ventana de la app |
| Apps empaquetadas (MSIX/UWP) | Notepad nuevo, Calculadora, WhatsApp | Se lanzan por AUMID; el proceso real puede ser hijo de `RuntimeBroker`/`ApplicationFrameHost.exe` (UWP) |
| Ejecutable con el mismo nombre en otra ruta | `chrome.exe` en `%TEMP%` | Se exige nombre **y** ubicación de confianza de la imagen ([WT-09](../04-security/threat-model.md#wt-09-malicious-application-impersonation)) |

Por eso cada entrada del catálogo declara `process_names` (y opcionalmente `image_paths` y `package_family_name`).

## Enumeración

| Necesidad | API | Notas |
|---|---|---|
| Procesos | `CreateToolhelp32Snapshot` + `Process32NextW` | Nombre, PID, PPID. Barato |
| Sesión del proceso | `ProcessIdToSessionId` | Filtrar a la sesión del agente (`WTSGetActiveConsoleSessionId` no: se usa la sesión **propia** del proceso) |
| Ruta de imagen | `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW` | Funciona con procesos de integridad alta (limited) salvo protegidos |
| Usuario del proceso | `OpenProcessToken` + `GetTokenInformation(TokenUser)` | Solo procesos con el SID del agente |
| Integridad | `GetTokenInformation(TokenIntegrityLevel)` | Detecta elevados → `APP_ELEVATED` |
| Paquete | `GetPackageFamilyName` | Apps MSIX |
| Ventanas | `EnumWindows` + `GetWindowThreadProcessId` + `IsWindowVisible` + `DwmGetWindowAttribute(DWMWA_CLOAKED)` + sin `WS_EX_TOOLWINDOW` + sin owner | "Ventana top-level visible real" (excluye ventanas ocultas de otros escritorios virtuales y fantasmas UWP) |

Nunca se leen línea de comandos (`PEB`), variables de entorno ni memoria de otros procesos.

## `open_app`

| `launch.kind` | Mecanismo | Resultado |
|---|---|---|
| `app_paths` / `path` | `ShellExecuteExW` con `SEE_MASK_NOCLOSEPROCESS`, verbo `open`, **sin** parámetros, directorio de trabajo = carpeta del ejecutable | `hProcess` del proceso lanzado |
| `aumid` | `IApplicationActivationManager::ActivateApplication(aumid, null, AO_NONE)` | PID del proceso activado |

Por qué `ShellExecuteExW` y no `CreateProcessW` para apps: respeta `App Paths`, manifiestos, shims de compatibilidad y el comportamiento que el usuario espera de "abrir la app". `CreateProcessW` se reserva para `run_command`, donde el control del entorno importa más.

`ERROR_ELEVATION_REQUIRED` (740) → `ACCESS_DENIED`, sin reintento con `runas` ([Privilege Model](../04-security/privilege-model.md)).

`ifRunning = "focus"`: `AllowSetForegroundWindow` no aplica a otro proceso; se usa `SetForegroundWindow` sobre la ventana principal. Windows puede negarse (foreground lock) → la verificación `foreground_is` decide. Por eso requiere el lock `DESKTOP_INPUT`.

## `close_app`

1. Enumerar procesos de la app en la sesión y del usuario; descartar la lista de procesos protegidos.
2. Si alguno es de integridad alta → `APP_ELEVATED` (no se toca ninguno: evita cierres parciales).
3. `PostMessageW(hwnd, WM_CLOSE)` a cada ventana top-level de esos procesos. Para UWP, a la ventana `ApplicationFrameWindow` asociada.
4. Esperar `gracePeriodMs` sondeando.
5. `force = true`: `OpenProcess(PROCESS_TERMINATE)` + `TerminateProcess(exit code 1)` sobre los restantes.
6. Verificación `process_absent`.

## Identificación de la app (implementado)

Un proceso pertenece a una entrada del catálogo si su nombre está en `process_names` **y** su imagen
está en la ruta exacta del catálogo o en una ubicación de confianza: `%SystemRoot%`,
`%ProgramFiles%`, `%ProgramFiles(x86)%` o el perfil del usuario, excluyendo siempre `%TEMP%`, `%TMP%`
y `Downloads`.

No se exige ruta idéntica porque Windows usa lanzadores y rutas por versión: `System32\notepad.exe`
abre el Notepad empaquetado bajo `WindowsApps`, y los navegadores se actualizan a carpetas nuevas.
Exigir igualdad estricta hacía fallar la verificación de `open_app` con Notepad en Windows 11
(comprobado en la máquina de desarrollo), mientras que el criterio de ubicación mantiene la
mitigación real: un `chrome.exe` descargado en `%TEMP%` no cuenta como la app del catálogo.

## Riesgos y trade-offs

| Riesgo | Mitigación |
|---|---|
| Apps que se minimizan a la bandeja en `WM_CLOSE` (Spotify, Teams, Discord) | La verificación fallará con `force=false` (correcto: la app no se cerró). El catálogo puede declarar `close_strategy = "terminate_after_close"` que convierte el cierre en R3 |
| App con diálogo "¿Guardar cambios?" | `VERIFICATION_FAILED` con mensaje explicativo; nunca se escala solo a `force` |
| PID reutilizado entre enumeración y acción | Se abre el handle una vez y se opera sobre él; se compara el tiempo de creación (`GetProcessTimes`) |
| Coste de enumerar ventanas con muchas apps | Una sola enumeración por sondeo, cache de 100 ms dentro de una verificación |
