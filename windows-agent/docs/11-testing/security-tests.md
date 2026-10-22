# Security Tests

Una prueba (o grupo) por amenaza mitigable del [Threat Model](../04-security/threat-model.md). Implementación en HU-WIN-046 … 049 (MVP) y HU-WIN-054, 057, 061–063 (post-MVP).

## Estado (implementado)

| Suite | Dónde | Qué cubre |
|---|---|---|
| Threat model | `crates/jarvis-agent/tests/threat_model.rs` | Una prueba por amenaza mitigable del MVP (WT-01, 02, 04, 05, 07, 09, 12, 14, 15) con el nombre de la amenaza en el test |
| Entradas hostiles | `crates/jarvis-protocol/tests/fuzz_parsers.rs` | 40 000 entradas aleatorias y mutadas más estructuras patológicas; la propiedad es que el parser nunca entra en pánico (WT-03) |
| Firmas y aprobación remota | `crates/jarvis-agent/tests/signatures.rs` | Firma ausente, ajena, alterada, reutilizada y repetida (WT-05, WT-15 en P8) |
| Updater | `crates/jarvis-agent/src/update.rs` (tests) | Manifiesto sin firma, con firma ajena, con campos alterados, downgrade y hash que no cuadra (WT-13) |
| Canarios y redacción | `crates/jarvis-agent/tests/` varios | Secreto y títulos de ventana fuera de logs, base y diagnóstico (WT-12, WT-14) |

## Matriz amenaza → test

| Amenaza | Test | HU | Fase |
|---|---|---|---|
| WT-01 Compromised Cloud | "Cloud malicioso" publica: R3 sin aprobación → nunca ejecuta; R4 → `NOT_PERMITTED`; principal desconocido → `NOT_PERMITTED`; `open_app` con `appId` fuera del catálogo → rechazo | HU-WIN-049 | MVP |
| WT-02 Stolen device credential | Conexión con la credencial del device intentando publicar en `down/tasks` → rechazada por el broker (ACL); sesión duplicada detectada | HU-WIN-049 | MVP |
| WT-03 Malicious MQTT message | Tabla de mensajes hostiles + fuzzing nocturno | HU-WIN-010, 047 | MVP |
| WT-04 Replay | Reenvío de un dispatch ya ejecutado (antes y después de *settled*) → sin re-ejecución; tras reset de BD → `STATE_RESET` | HU-WIN-049, 043 | MVP |
| WT-05 Task tampering | Payload con `force` alterado → riesgo R3 → aprobación con el input real mostrado | HU-WIN-049 | MVP (firma en P8) |
| WT-06 Path traversal | Ver [Filesystem](#filesystem) | HU-WIN-054 | P7 |
| WT-07 Command injection | Metacaracteres en `appId` y en parámetros de comandos; `open_app` con argumentos → `INVALID_PAYLOAD`; property test del escapado | HU-WIN-049, 057 | MVP / P7 |
| WT-08 Privilege escalation | Arranque elevado rechazado; `ERROR_ELEVATION_REQUIRED` sin `runas`; orden de búsqueda de DLL | HU-WIN-048 | MVP |
| WT-09 App impersonation | `chrome.exe` falso en `%TEMP%` no cuenta como `chrome` cuando la entrada tiene ruta resuelta | HU-WIN-049 | MVP |
| WT-10 Symlink/junction | Ver [Filesystem](#filesystem) | HU-WIN-054 | P7 |
| WT-11 Local attacker | Pipe inaccesible para otro usuario; ACL de directorios; sin puertos en escucha | HU-WIN-048 | MVP |
| WT-12 Credential extraction | Secreto no presente en config, BD, logs, diagnósticos, línea de comandos; `Debug` redactado | HU-WIN-046 | MVP |
| WT-13 Update compromise | Manifiesto con firma inválida, hash alterado, downgrade → rechazo | HU-WIN-063 | P8 |
| WT-14 Sensitive logs | Ver [Datos sensibles](#datos-sensibles) | HU-WIN-046 | MVP |
| WT-15 Approval spoofing | Mensaje de "aprobación" por MQTT ignorado (MVP); aprobación firmada falsificada rechazada (P8) | HU-WIN-049, 062 | MVP / P8 |

## Datos sensibles

Canarios únicos por ejecución (`CANARY-<uuid>`) insertados en:

| Origen | Dónde no debe aparecer |
|---|---|
| Device secret | Logs, `config.toml`, BD, `diagnostics`, mensajes MQTT, línea de comandos del proceso |
| Título de ventana (`jarvis-test-app` con título canario) | Logs; BD tras *settled* (salvo en el `result` antes del settle); `diagnostics` |
| Contenido de archivo (P7) | Logs; BD tras *settled*; `diagnostics` |
| Salida de comando (P7) | Logs; BD tras *settled*; `diagnostics` |

Búsqueda en UTF-8 y UTF-16LE en todos los archivos bajo `%LOCALAPPDATA%\Jarvis\Agent\` y en el zip de diagnóstico.

## Filesystem

Rutas que deben rechazarse (`PATH_NOT_ALLOWED` o `INVALID_PAYLOAD`) con raíz permitida `C:\JarvisTest\allowed`:

| Caso | Ejemplo |
|---|---|
| Traversal | `C:\JarvisTest\allowed\..\secret.txt` |
| Prefijo engañoso | `C:\JarvisTest\allowed2\x.txt` |
| Relativa | `..\secret.txt`, `secret.txt` |
| Namespace | `\\?\C:\JarvisTest\secret.txt`, `\\.\C:\JarvisTest\secret.txt` |
| UNC | `\\127.0.0.1\c$\JarvisTest\secret.txt`, `\\attacker\share\x` |
| Dispositivo | `C:\JarvisTest\allowed\CON`, `NUL.txt`, `\\.\PhysicalDrive0` |
| ADS | `C:\JarvisTest\allowed\a.txt:hidden` |
| Junction | `allowed\link` → `C:\JarvisTest\secret` |
| Symlink | `allowed\s.txt` → `C:\JarvisTest\secret.txt` |
| TOCTOU | Directorio reemplazado por junction entre validación y apertura |
| Short name | `C:\JARVIS~1\allowed\..\secret.txt` |
| Unidad de red | Unidad mapeada `Z:\` |
| Extensión sensible | `allowed\id_rsa`, `allowed\cert.pfx` |
| Oculto/sistema | Archivo con atributo `HIDDEN` |

## Revisión manual por release

- Revisar `policy.toml` por defecto: sin principals remotos.
- `cargo deny` / advisories.
- Revisión de todo bloque `unsafe` nuevo.
