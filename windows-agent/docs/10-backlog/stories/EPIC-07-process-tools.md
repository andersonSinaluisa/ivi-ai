# EPIC-07 — Process Tools

Fase **P5**. Objetivo: las cinco tools del MVP. Diseño en [Process Tools](../../03-tools/process-tools.md) y [Process Management](../../05-windows/process-management.md). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-028 — Plataforma: procesos y ventanas

**Goal**: observar procesos y ventanas de la sesión del usuario de forma segura y testeable.

**Description**: interfaces `ProcessApi` y `WindowApi` en `jarvis-windows` con fakes, según la tabla de [Enumeración](../../05-windows/process-management.md#enumeración).

**Acceptance Criteria**
1. Enumerar procesos con nombre, PID, PPID, sesión, hora de creación, ruta de imagen (si es accesible), integridad y si pertenecen al SID del agente.
2. Enumerar ventanas top-level "reales" (visibles, no cloaked, no tool windows, sin owner) con PID y clase.
3. Solo se devuelven procesos de la sesión del agente y del mismo usuario (filtro por defecto, no opcional para las tools).
4. Handles RAII; ninguna fuga tras 10 000 enumeraciones (test).
5. Nunca se lee línea de comandos, entorno ni memoria de otros procesos.
6. Fakes con escenarios programables (proceso que aparece tarde, que sale, elevado, ventana cloaked).

**Dependencies**: HU-WIN-001.

**Technical Considerations**: `spawn_blocking`; `QueryFullProcessImageNameW` con `PROCESS_QUERY_LIMITED_INFORMATION`; `DwmGetWindowAttribute(DWMWA_CLOAKED)`.

**Tests**: integración Windows: lanzar `notepad.exe` de prueba y encontrarlo con su ventana; contador de handles antes/después.

**Definition of Done**: DoD común.

---

## HU-WIN-029 — Catálogo de aplicaciones

**Goal**: resolver `appId` a una forma segura de lanzar e identificar la app.

**Description**: sección `[apps.*]` de `policy.toml` y resolución de [Tool Registry](../../03-tools/tool-registry.md#catálogo-de-aplicaciones-appid).

**Acceptance Criteria**
1. Entradas por defecto: `chrome`, `edge`, `notepad`, `explorer`, `calculator`; el owner las extiende o sobreescribe.
2. `launch.kind`: `app_paths` (HKCU luego HKLM), `aumid`, `path` (absoluta, variables expandidas).
3. `path` en carpeta escribible por otros usuarios o inexistente → la entrada se marca inválida (log `WARN`), no se usa.
4. `appId` no presente → `APP_NOT_FOUND` en validación (→ `task.rejected INVALID_PAYLOAD`).
5. Resolución cacheada con invalidación al recargar la política.

**Dependencies**: HU-WIN-024.

**Technical Considerations**: lectura de registro con `RegGetValueW`; comprobación de ACL de la carpeta con `GetNamedSecurityInfoW`.

**Tests**: unit de parsing; integración Windows: `chrome` por App Paths (si está instalado en el runner, si no `notepad`).

**Definition of Done**: DoD común.

---

## HU-WIN-030 — Tool `is_app_running`

**Goal**: saber si una app del catálogo está en ejecución.

**Description**: [is_app_running](../../03-tools/process-tools.md#is_app_running--r0).

**Acceptance Criteria**
1. Input `{ appId }`; output `{ appId, running, processCount }`.
2. R0, `windows.apps.read`, sin locks, timeouts 2 s / 5 s, verificación `NOT_APPLICABLE`.
3. Cuenta solo procesos de la sesión y del usuario; identifica por `process_names` y, si la entrada tiene ruta resuelta, por ruta de imagen.
4. Funciona con la sesión bloqueada.

**Dependencies**: HU-WIN-028, HU-WIN-029, HU-WIN-023.

**Technical Considerations**: reutilizar el matcher de procesos que usarán las verificaciones (HU-WIN-036).

**Tests**: unit con fakes; integración Windows con `notepad`.

**Definition of Done**: DoD común.

---

## HU-WIN-031 — Tool `list_running_apps`

**Goal**: observar qué apps están abiertas sin exponer datos sensibles.

**Description**: [list_running_apps](../../03-tools/process-tools.md#list_running_apps--r0).

**Acceptance Criteria**
1. Input `{ includeUncatalogued = false, limit = 100 }`.
2. Por defecto solo apps del catálogo; con `includeUncatalogued` añade nombres de ejecutable con ventana visible.
3. Sin rutas, líneas de comandos, PIDs de otros usuarios ni procesos de sistema/servicios.
4. Output ≤ 24 KiB; si se excede, `truncated: true`.

**Dependencies**: HU-WIN-030.

**Technical Considerations**: agrupar por `appId` o por nombre de ejecutable.

**Tests**: unit con 500 procesos simulados (truncado); integración Windows.

**Definition of Done**: DoD común.

---

## HU-WIN-032 — Tool `get_active_window`

**Goal**: conocer la ventana activa respetando la privacidad.

**Description**: [get_active_window](../../03-tools/process-tools.md#get_active_window--r1) y [Active Window](../../05-windows/active-window.md).

**Acceptance Criteria**
1. Output `{ appId?, process, pid, title?, titleRedacted, sessionState }`.
2. Título solo con `includeTitle = true` **y** capability `windows.window.title`; si falta la capability → `NOT_PERMITTED`.
3. Reglas de privacidad: `never_title`, modo privado de navegadores, `title_mode`, redacción de patrones (emails, números largos, URLs con query), máx. 256 caracteres.
4. Sesión bloqueada → `{ sessionState: "Locked" }` sin error.
5. El título nunca aparece en logs (test canario); `sensitivity = output`.

**Dependencies**: HU-WIN-028, HU-WIN-011, HU-WIN-026.

**Technical Considerations**: `GetWindowTextW` puede bloquear con ventanas colgadas → usar `InternalGetWindowText` o `SendMessageTimeoutW(WM_GETTEXT)` con timeout.

**Tests**: unit de redacción con tabla de títulos; integración Windows con una ventana de título canario.

**Definition of Done**: DoD común.

---

## HU-WIN-033 — Tool `open_app` (+ alias `OPEN_CHROME`)

**Goal**: abrir una app del catálogo y confirmar que realmente está abierta.

**Description**: [open_app](../../03-tools/process-tools.md#open_app--r2).

**Acceptance Criteria**
1. Input `{ appId, ifRunning = "noop" | "focus" | "new_instance" }`; sin argumentos ni URLs.
2. R2, `windows.apps.open`, lock `APP:<appId>` (+ `DESKTOP_INPUT` con `focus`), timeouts 15 s / 60 s, cancelación cooperativa.
3. Lanzamiento por `ShellExecuteExW` (App Paths/path) o `IApplicationActivationManager` (AUMID).
4. `ERROR_ELEVATION_REQUIRED` → `ACCESS_DENIED` sin reintento con `runas`.
5. Sesión `Locked`/`Disconnected` → `SESSION_UNAVAILABLE`.
6. Verificación `process_and_window` (HU-WIN-036); app que sale al instante → `VERIFICATION_FAILED` con código de salida.
7. `OPEN_CHROME` produce exactamente el mismo resultado que `OPEN_APP {appId: "chrome"}`.

**Dependencies**: HU-WIN-029, HU-WIN-035, HU-WIN-036.

**Technical Considerations**: la activación de apps empaquetadas requiere COM en el hilo STA; `SEE_MASK_NOCLOSEPROCESS` para obtener el handle.

**Tests**: integración Windows: abrir `notepad` (PASSED); app de prueba que sale con código 1 (FAILED); `ifRunning=noop` con app abierta.

**Definition of Done**: DoD común + [WIN-E2E-01](../../11-testing/integration-scenarios.md#win-e2e-01-tarea-válida-open_app) y [WIN-E2E-08](../../11-testing/integration-scenarios.md#win-e2e-08-verificación-fallida) verdes.

---

## HU-WIN-034 — Tool `close_app`

**Goal**: cerrar una app de forma segura y confirmar que se cerró.

**Description**: [close_app](../../03-tools/process-tools.md#close_app--r2-r3-con-force) y [Process Management](../../05-windows/process-management.md#close_app).

**Acceptance Criteria**
1. Input `{ appId, force = false, gracePeriodMs = 5000 }`; `force` eleva a R3 y exige `windows.apps.close.force`.
2. `WM_CLOSE` a las ventanas top-level; espera `gracePeriodMs`; con `force` → `TerminateProcess` de los restantes.
3. Sin `force` y la app sigue viva → `VERIFICATION_FAILED` explicativo, sin escalar.
4. Algún proceso elevado → `APP_ELEVATED` sin tocar ninguno.
5. Lista de procesos protegidos nunca se cierra; solo procesos del SID del usuario.
6. App no en ejecución → éxito con `wasRunning: false`.
7. Verificación `process_absent`.

**Dependencies**: HU-WIN-030, HU-WIN-035, HU-WIN-036, HU-WIN-027.

**Technical Considerations**: comparar hora de creación para evitar PID reutilizado; UWP vía `ApplicationFrameWindow`.

**Tests**: integración Windows: `notepad` sin cambios (PASSED); `notepad` con texto sin guardar (FAILED sin force; PASSED con force aprobado); intento sobre `explorer.exe` rechazado.

**Definition of Done**: DoD común + [WIN-E2E-11](../../11-testing/integration-scenarios.md#win-e2e-11-cancelación) y [WIN-E2E-13](../../11-testing/integration-scenarios.md#win-e2e-13-aprobación-local-de-una-acción-r3) verdes.
