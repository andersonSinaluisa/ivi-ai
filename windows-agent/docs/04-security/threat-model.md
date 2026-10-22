# Threat Model — Windows Agent

Complementa el [Threat Model del Cloud](../../../api/docs/05-security/threat-model.md). Fronteras en [Trust Boundaries](../01-architecture/trust-boundaries.md).

**Activos**: integridad y disponibilidad de la PC del usuario, datos personales (archivos, títulos, apps en uso), trabajo no guardado, credencial del device, integridad del propio agente.

**Escala**: Likelihood / Impact = Baja · Media · Alta. Residual = riesgo aceptado tras las mitigaciones.

## Resumen

| ID | Amenaza | Likelihood | Impact | Residual |
|---|---|---|---|---|
| WT-01 | Compromised Cloud | Media | Alta | Media |
| WT-02 | Stolen device credential | Media | Media | Baja |
| WT-03 | Malicious MQTT message | Baja | Alta | Baja |
| WT-04 | Replay | Media | Media | Baja |
| WT-05 | Task tampering | Baja | Alta | Media |
| WT-06 | Path traversal | Media | Alta | Baja |
| WT-07 | Command injection | Media | Alta | Baja |
| WT-08 | Privilege escalation | Baja | Alta | Baja |
| WT-09 | Malicious application impersonation | Media | Media | Media |
| WT-10 | Symlink/junction attacks | Media | Alta | Baja |
| WT-11 | Local attacker | Media | Alta | Alta (fuera del modelo) |
| WT-12 | Credential extraction | Media | Media | Media |
| WT-13 | Agent update compromise | Baja | Alta | Baja (tras P8) |
| WT-14 | Sensitive logs | Alta | Media | Baja |
| WT-15 | Approval spoofing | Media | Alta | Baja |

## WT-01 Compromised Cloud

| | |
|---|---|
| **Threat** | Un atacante controla la API o el broker del Cloud (RCE en el VPS, credencial de admin robada) |
| **Attack** | Publica `task.dispatch` arbitrarios en `down/tasks` con `requestedBy` elegido (p. ej. el owner) |
| **Likelihood** | Media: el VPS es un único servidor expuesto a Internet |
| **Impact** | Alta sin mitigaciones: control remoto de la PC |
| **Mitigation** | Autoridad local ([ADR-004](../09-decisions/ADR-004-local-permission-authority.md)); sin shell ni rutas/argumentos remotos ([ADR-008](../09-decisions/ADR-008-restricted-command-execution.md)); catálogo de apps local; R3 con aprobación **local**; R4 remoto siempre denegado; política por defecto sin principals remotos; (P8) firma end-to-end de tareas y aprobaciones con claves que no están en el Cloud |
| **Residual Risk** | Media: hasta P8 el atacante puede ejecutar todo lo R0–R2 concedido a cualquier principal (abrir/cerrar apps del catálogo sin `force`, observar apps). Sin pérdida de datos ni ejecución arbitraria |

## WT-02 Stolen device credential

| | |
|---|---|
| **Threat** | Robo del device secret del Windows Agent |
| **Attack** | Conectarse al broker como el device: publicar presencia falsa, resultados falsos de tareas, provocar DoS por *session takeover* (mismo client id) |
| **Likelihood** | Media (malware, backup en claro de otra herramienta) |
| **Impact** | Media: engaña al owner sobre el estado; **no** permite enviar tareas al agente (ACL: solo `jarvis-cloud` publica en `down/#`) |
| **Mitigation** | Credential Manager ([Credential Storage](credential-storage.md)); nunca en archivos ni argumentos; el Cloud detecta sesiones concurrentes (`sessionId` distinto) y el owner revoca/rota |
| **Residual Risk** | Baja |

## WT-03 Malicious MQTT message

| | |
|---|---|
| **Threat** | Mensaje malformado o hostil que explota el parser o la lógica |
| **Attack** | JSON gigante o anidado, campos inesperados, tipos incorrectos, `deviceId` ajeno, unicode malicioso en `appId` |
| **Likelihood** | Baja (requiere capacidad de publicar en `down/`) |
| **Impact** | Alta si provoca crash en bucle o evasión de validación |
| **Mitigation** | Límite 64 KiB antes de parsear; `serde` con `deny_unknown_fields` y límites de profundidad; validación de envelope, topic y `deviceId`; esquemas estrictos con patrones (`^[a-z0-9_-]{1,64}$`); Rust memory-safe; *fuzzing* del parser (EPIC-11); un mensaje inválido nunca tumba el proceso |
| **Residual Risk** | Baja |

## WT-04 Replay

| | |
|---|---|
| **Threat** | Reenvío de un `task.dispatch` legítimo antiguo |
| **Attack** | Capturar/reinyectar un dispatch (desde el broker o un Cloud comprometido) para repetir una acción |
| **Likelihood** | Media (QoS 1 ya produce redeliveries legítimos) |
| **Impact** | Media: repetir acciones (reabrir apps) |
| **Mitigation** | Dedup por `taskId` en SQLite (nunca se re-ejecuta, [Idempotency](../06-state/idempotency.md)); `expiresAt` con reloj corregido; retención de 8 días posterior a la expiración máxima; guardia de reinicio de BD (`STATE_RESET`); (P8) firma con `taskId` + `expiresAt` |
| **Residual Risk** | Baja: un replay con `taskId` **nuevo** no es replay sino inyección (WT-01/WT-05) |

## WT-05 Task tampering

| | |
|---|---|
| **Threat** | Modificación del contenido de una tarea entre el originador y el agente |
| **Attack** | Cambiar `payload` (`appId: chrome` → `appId: vscode`, `force: false` → `true`) o `requestedBy` en el Cloud |
| **Likelihood** | Baja (TLS en ambos tramos; requiere comprometer el Cloud) |
| **Impact** | Alta |
| **Mitigation** | TLS verificado; riesgo recalculado localmente sobre el payload recibido (`force=true` → R3 → aprobación local que muestra el input real); (P8) firma del originador sobre payload |
| **Residual Risk** | Media hasta P8 (misma superficie que WT-01), Baja después |

## WT-06 Path traversal

| | |
|---|---|
| **Threat** | Leer archivos fuera de las raíces permitidas |
| **Attack** | `..\..\Users\x\.ssh\id_rsa`, rutas absolutas, `\\?\C:\`, mayúsculas/minúsculas, nombres 8.3 |
| **Likelihood** | Media (ataque clásico) |
| **Impact** | Alta (exfiltración) |
| **Mitigation** | Algoritmo de [Filesystem Tools](../03-tools/filesystem-tools.md): rechazo de formas peligrosas, normalización, comparación por componentes, ruta final real del handle; sin raíces por defecto; extensiones sensibles denegadas; R1 con capability explícita; tests de seguridad dedicados |
| **Residual Risk** | Baja (y solo desde P7) |

## WT-07 Command injection

| | |
|---|---|
| **Threat** | Ejecutar comandos arbitrarios |
| **Attack** | `"; del /s …"`, `& calc`, `$(…)`, `--load-extension=` en `open_app`, parámetros con metacaracteres en `run_command` |
| **Likelihood** | Media |
| **Impact** | Alta (RCE) |
| **Mitigation** | Sin shell ([ADR-008](../09-decisions/ADR-008-restricted-command-execution.md)); `open_app` sin argumentos; `run_command` solo `commandId` del catálogo local con parámetros tipados y con patrón; `CreateProcessW` con ejecutable absoluto y escapado por argumento; intérpretes prohibidos como ejecutable; Job Objects |
| **Residual Risk** | Baja: queda el riesgo de que el owner defina un comando peligroso (decisión local) |

## WT-08 Privilege escalation

| | |
|---|---|
| **Threat** | Usar el agente para obtener privilegios superiores |
| **Attack** | Lanzar una app con `runas`; explotar DLL search order en `open_app`; escribir en ubicaciones que un proceso elevado carga; usar el agente como *confused deputy* |
| **Likelihood** | Baja |
| **Impact** | Alta |
| **Mitigation** | Integridad media, sin `runas`, sin elevación ([Privilege Model](privilege-model.md)); rutas absolutas y `App Paths` (no búsqueda en el directorio actual); `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` en el agente; instalación en `%LOCALAPPDATA%` sin permisos de escritura de otros usuarios; rechazo de arranque elevado |
| **Residual Risk** | Baja |

## WT-09 Malicious application impersonation

| | |
|---|---|
| **Threat** | Una app maliciosa se hace pasar por una del catálogo o por una ventana conocida |
| **Attack** | Un `chrome.exe` en `%TEMP%` que se ejecuta y hace que `is_app_running`/verificación lo cuente; una ventana con título falso para `get_active_window`; un proceso con nombre `Spotify.exe` que recibe `close_app` |
| **Likelihood** | Media |
| **Impact** | Media: resultados de verificación incorrectos; `close_app` cierra un proceso equivocado (del mismo usuario) |
| **Mitigation** | Nombre del ejecutable **más** ubicación de la imagen (`QueryFullProcessImageNameW`): ruta exacta del catálogo o carpeta de confianza, nunca `%TEMP%`/`Downloads` ([Process Management](../05-windows/process-management.md#identificación-de-la-app-implementado)); solo procesos del SID del usuario; (futuro) firma Authenticode; lista de procesos protegidos |
| **Residual Risk** | Media: malware del mismo usuario puede engañar a la observación (fuera del modelo, ver WT-11) |

## WT-10 Symlink/junction attacks

| | |
|---|---|
| **Threat** | Redirigir una ruta permitida hacia otra no permitida |
| **Attack** | Crear en `Documents\Jarvis` un junction a `C:\Users\x\AppData` o un symlink a otro volumen; cambiar el enlace entre validación y lectura (TOCTOU) |
| **Likelihood** | Media (crear junctions no requiere admin) |
| **Impact** | Alta |
| **Mitigation** | `FILE_FLAG_OPEN_REPARSE_POINT`, rechazo de reparse points, `GetFinalPathNameByHandleW` + revalidación, operación sobre el handle ya abierto |
| **Residual Risk** | Baja |

## WT-11 Local attacker

| | |
|---|---|
| **Threat** | Proceso u persona local que usa el agente |
| **Attack** | Otro proceso del usuario habla con el named pipe, edita `policy.toml`, lee `agent.db`; otro usuario de la PC intenta hablar con el agente |
| **Likelihood** | Media |
| **Impact** | Alta |
| **Mitigation** | Otro **usuario**: pipe con ACL del SID, archivos en el perfil con ACL del usuario, Credential Manager por usuario → bloqueado. Mismo usuario: fuera del modelo (tiene el mismo poder sin el agente); se registra en logs cada cambio de política y cada aprobación local |
| **Residual Risk** | Alta para el mismo usuario (aceptado y documentado); Baja para otros usuarios |

## WT-12 Credential extraction

| | |
|---|---|
| **Threat** | Extracción del secreto de Credential Manager o de la memoria |
| **Attack** | `CredReadW` desde malware del usuario; volcado de memoria del proceso; crash dump con el secreto |
| **Likelihood** | Media |
| **Impact** | Media (ver WT-02) |
| **Mitigation** | DPAPI por usuario, `CRED_PERSIST_LOCAL_MACHINE`; `zeroize` de buffers propios; los crash dumps del agente se desactivan (`WerAddExcludedApplication` / sin minidumps propios con memoria completa); futuro mTLS con clave no exportable/TPM |
| **Residual Risk** | Media |

## WT-13 Agent update compromise

| | |
|---|---|
| **Threat** | Distribuir un binario malicioso como actualización |
| **Attack** | Comprometer el canal de actualización o el Cloud para servir un `jarvis-agent.exe` troyanizado |
| **Likelihood** | Baja |
| **Impact** | Alta (código arbitrario con la cuenta del usuario, persistente) |
| **Mitigation** | MVP: sin updater automático (actualización manual). P8: manifiesto firmado con Ed25519 (clave offline, pública embebida), hash SHA-256, Authenticode, protección anti-downgrade (versión monotónica), rollback a la versión anterior si el nuevo binario no reporta health en N segundos ([Startup Model](../05-windows/startup-model.md#actualizaciones)) |
| **Residual Risk** | Baja tras P8 (depende de la custodia de la clave offline) |

## WT-14 Sensitive logs

| | |
|---|---|
| **Threat** | Datos sensibles en logs, diagnósticos o la BD local |
| **Attack** | Leer los logs (soporte, backup, otro programa) y obtener secretos, contenido de archivos, títulos de ventanas, salida de comandos |
| **Likelihood** | Alta (los logs se comparten al pedir ayuda) |
| **Impact** | Media |
| **Mitigation** | Reglas de [Logging](../07-observability/logging.md): nunca passwords/tokens/contenido/clipboard/salida; campos sensibles marcados en tipos (`Sensitive<T>` que no implementa `Display`); redacción post-settle en BD; diagnóstico con redacción automática; tests que buscan canarios en logs |
| **Residual Risk** | Baja |

## WT-15 Approval spoofing

| | |
|---|---|
| **Threat** | Obtener una aprobación sin decisión real del owner |
| **Attack** | Un Cloud comprometido envía una "aprobación"; una app muestra una notificación falsa que induce a aprobar; otro proceso llama a la CLI; reutilizar una aprobación para otra tarea |
| **Likelihood** | Media |
| **Impact** | Alta (desbloquea R3) |
| **Mitigation** | MVP: aprobación solo local por named pipe con ACL; el Cloud no puede aprobar. P8: aprobaciones firmadas por la clave del dispositivo del owner, vinculadas a `taskId` + hash del payload + nonce; toasts sin botones hasta tener UI firmada ([Approvals](approvals.md)) |
| **Residual Risk** | Baja (malware del mismo usuario queda en WT-11) |

## Nota sobre el alcance

La solicitud pedía un modelo de amenazas; se analizan las **15** amenazas listadas (no 14). Las de "Cloud" que no afectan al agente (brute force de la API, cross-user) están en el threat model del Cloud.
