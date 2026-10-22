# EPIC-13 — Post-MVP Tools (filesystem, comandos, foco, notificaciones)

Fase **P7**. Objetivo: capacidades de mayor riesgo, con sandbox y pruebas de seguridad propias. Requiere el MVP completo (HU-WIN-053). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

> **Estado: implementada.** Ver [Estado de implementación](../implementation-status.md). La cancelación de `run_command` mata el árbol de procesos de verdad (bandera de cancelación hasta el Job Object).

---

## HU-WIN-054 — Sandbox de rutas

**Goal**: validar rutas de forma que ninguna lectura escape de las raíces permitidas.

**Description**: [Algoritmo de validación de ruta](../../03-tools/filesystem-tools.md#algoritmo-de-validación-de-ruta) (mitiga [WT-06](../../04-security/threat-model.md#wt-06-path-traversal) y [WT-10](../../04-security/threat-model.md#wt-10-symlinkjunction-attacks)).

**Acceptance Criteria**
1. Rechazo de rutas relativas, `\\?\`, `\\.\`, UNC, nombres de dispositivo, ADS, caracteres de control y longitud > 1024.
2. Normalización y comparación por componentes, sin distinguir mayúsculas.
3. Apertura sin seguir reparse points; rechazo de symlinks, junctions y mount points.
4. Revalidación con `GetFinalPathNameByHandleW`; las operaciones posteriores usan el handle.
5. Solo unidades `DRIVE_FIXED`; archivos ocultos/sistema excluidos salvo configuración.
6. Sin raíces configuradas → toda ruta `PATH_NOT_ALLOWED`.

**Dependencies**: HU-WIN-053, HU-WIN-024.

**Technical Considerations**: API `SandboxedPath` que solo puede construirse por el validador (tipo opaco).

**Tests**: tabla de ≥ 50 rutas hostiles; junction y symlink creados en el test (el symlink requiere modo desarrollador o privilegio; si no está disponible, test marcado); carrera TOCTOU que cambia un directorio por un junction entre validación y lectura.

**Definition of Done**: DoD común + casos de filesystem de [Security Tests](../../11-testing/security-tests.md#filesystem) verdes.

---

## HU-WIN-055 — Tool `list_directory`

**Goal**: listar el contenido de una carpeta permitida.

**Description**: [list_directory](../../03-tools/filesystem-tools.md#list_directory--r1).

**Acceptance Criteria**
1. Input `{ path, limit ≤ 500 }`; output con `name`, `kind`, `size`, `modifiedAt`, `truncated`.
2. R1, `filesystem.list` + raíz con `list = true`; lock `FS:<ruta>` de lectura.
3. No recursivo; ocultos/sistema excluidos.
4. Nombres nunca en logs; `sensitivity = output`.

**Dependencies**: HU-WIN-054.

**Technical Considerations**: `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)` sobre el handle validado.

**Tests**: integración con un árbol temporal.

**Definition of Done**: DoD común.

---

## HU-WIN-056 — Tool `read_file`

**Goal**: leer archivos de texto permitidos sin riesgo de exfiltración masiva.

**Description**: [read_file](../../03-tools/filesystem-tools.md#read_file--r1).

**Acceptance Criteria**
1. Input `{ path, offset, maxBytes ≤ 24576 }`; output `{ encoding, content, size, sha256, truncated }`.
2. Solo UTF-8/UTF-16 con BOM; binario → `BINARY_FILE`.
3. Extensiones sensibles denegadas por defecto.
4. Contenido nunca en logs; redactado tras *settled*.
5. R1, `filesystem.read` + raíz con `read = true`.

**Dependencies**: HU-WIN-054.

**Technical Considerations**: lectura sobre el handle validado; `sha256` del archivo completo solo si ≤ `max_read_bytes`.

**Tests**: canario de contenido (HU-WIN-046 ampliada); archivos grandes, binarios, extensiones denegadas.

**Definition of Done**: DoD común.

---

## HU-WIN-057 — Catálogo de comandos y lanzador con Job Object

**Goal**: ejecutar procesos definidos localmente de forma aislada y controlada.

**Description**: [Command Execution](../../03-tools/command-execution.md#reglas-de-ejecución) (mitiga [WT-07](../../04-security/threat-model.md#wt-07-command-injection)).

**Acceptance Criteria**
1. Sección `[commands."<id>"]` validada: ejecutable absoluto existente, no prohibido, parámetros tipados con patrón anclado, riesgo R1–R3.
2. Construcción de la línea de comandos con escapado por argumento (reglas de `CommandLineToArgvW`); ningún intérprete.
3. `CreateProcessW` con `lpApplicationName`, entorno mínimo, directorio fijo, sin herencia salvo pipes.
4. Job Object con kill-on-close, límite de memoria y de procesos; cancelación/timeout = `TerminateJobObject`.
5. Salida capturada según configuración y truncada.

**Dependencies**: HU-WIN-053.

**Technical Considerations**: test de propiedad del escapado: para cadenas aleatorias, `CommandLineToArgvW(escape(args)) == args`.

**Tests**: property test del escapado; inyección con metacaracteres; proceso que crea hijos y es cancelado.

**Definition of Done**: DoD común.

---

## HU-WIN-058 — Tool `run_command`

**Goal**: exponer los comandos del catálogo como tool.

**Description**: [run_command](../../03-tools/command-execution.md#run_command).

**Acceptance Criteria**
1. Input `{ commandId, params }`; `COMMAND_NOT_FOUND` si no existe.
2. Permisos `commands.run` y `commands.run:<id>`; riesgo del comando.
3. Lock `COMMAND:<id>`; límite global de 2 comandos concurrentes.
4. Verificación `exit_code_expected`.
5. Salida redactada tras *settled* y nunca en logs.

**Dependencies**: HU-WIN-057, HU-WIN-026.

**Technical Considerations**: la descripción del comando se incluye en `tools list` para el futuro Agent Core.

**Tests**: integración con `ipconfig /flushdns` (o un ejecutable de prueba propio) y códigos de salida esperados/inesperados.

**Definition of Done**: DoD común.

---

## HU-WIN-059 — Tool `focus_window`

**Goal**: traer una app al primer plano y confirmarlo.

**Description**: `focus_window` de [Tool System](../../03-tools/tool-system.md#catálogo).

**Acceptance Criteria**
1. Input `{ appId }`; R2, `windows.window.focus`, lock `DESKTOP_INPUT`.
2. Restaurar si está minimizada; `SetForegroundWindow`; verificación `foreground_is`.
3. Windows niega el foco (foreground lock) → `VERIFICATION_FAILED` explicativo.
4. Sesión bloqueada → `SESSION_UNAVAILABLE`.

**Dependencies**: HU-WIN-053.

**Technical Considerations**: el agente no es la app en primer plano: documentar la técnica elegida (p. ej. `AllowSetForegroundWindow` no aplica); no usar `AttachThreadInput` con procesos de mayor integridad.

**Tests**: integración en el runner interactivo.

**Definition of Done**: DoD común.

---

## HU-WIN-060 — Tool `show_notification`

**Goal**: mostrar una notificación al usuario (TaskType `SHOW_NOTIFICATION` del Cloud, [C15](../../00-overview/cloud-compatibility.md)).

**Description**: toast de Windows con título y cuerpo.

**Acceptance Criteria**
1. Input `{ title ≤ 64, body ≤ 256 }` (texto plano, sin XML inyectable: se escapa).
2. R1, `windows.notify`; verificación: la API de notificaciones confirma la entrega (`ToastNotifier` sin excepción y estado `Shown` cuando el evento esté disponible), si no `INCONCLUSIVE`.
3. Sin botones ni acciones que ejecuten algo.
4. Si las notificaciones están desactivadas por el usuario → `EXECUTION_FAILED` con mensaje claro.

**Dependencies**: HU-WIN-053.

**Technical Considerations**: una app sin paquete necesita un AUMID registrado mediante acceso directo en el menú Inicio (lo crea `install`).

**Tests**: integración en el runner interactivo.

**Definition of Done**: DoD común.
