# Filesystem Tools (P7, post-MVP)

`list_directory` y `read_file`. Solo lectura. Escritura/borrado quedan para el futuro con riesgo R3/R4.

## Por qué no en el MVP

Leer archivos es la capacidad con **mayor potencial de exfiltración**: un Cloud comprometido podría leer `.ssh`, perfiles del navegador, documentos. Antes de exponerla deben existir: sandbox de rutas probado, redacción post-settle, auditoría local y el flujo de aprobaciones.

## Allowed roots

```toml
# policy.toml
[filesystem]
allowed_roots = [
  { path = "%USERPROFILE%\\Documents\\Jarvis", read = true, list = true },
  { path = "%USERPROFILE%\\Downloads", list = true, read = false },
]
max_read_bytes = 262144        # 256 KiB leídos; se devuelven ≤ 24 KiB
max_list_entries = 500
include_hidden = false
```

Por defecto **no hay ninguna raíz permitida**: las tools de filesystem están deshabilitadas hasta que el owner configure raíces localmente.

## Algoritmo de validación de ruta

Orden estricto (en `validate`, antes de la política):

1. **Rechazar formas peligrosas del input**: rutas relativas, prefijos `\\?\` y `\\.\`, UNC (`\\server\share`), nombres de dispositivo (`CON`, `NUL`, `COM1`, `LPT1`…, también con extensión), ADS (`archivo:stream`), caracteres de control, longitud > 1024.
2. **Normalizar** léxicamente (`GetFullPathNameW`), eliminando `.` y `..`.
3. **Comprobar prefijo léxico** contra allowed roots (comparación case-insensitive por componentes, no por string: `C:\Docs2` no está dentro de `C:\Docs`).
4. **Abrir** con `CreateFileW` (`FILE_FLAG_BACKUP_SEMANTICS` para directorios, acceso de lectura, compartición de lectura) **sin** seguir reparse points en el último componente (`FILE_FLAG_OPEN_REPARSE_POINT`) y comprobar que no sea symlink/junction/mount point.
5. **Ruta final real**: `GetFinalPathNameByHandleW` del handle abierto y **volver a comprobar** que está dentro de una raíz permitida (cubre symlinks/junctions intermedios).
6. Operar **sobre el handle** ya abierto (no reabrir por ruta) → evita TOCTOU (cambio de la ruta entre validación y uso).
7. Rechazar volúmenes de red y unidades mapeadas (`GetDriveTypeW` ≠ `DRIVE_FIXED`) salvo configuración explícita futura.

| Amenaza | Mitigación |
|---|---|
| Path traversal `..\..\Windows` | Normalización + prefijo por componentes (2–3) |
| Symlink / junction hacia fuera | No seguir reparse points + ruta final (4–5) |
| TOCTOU | Operar sobre handle (6) |
| UNC → NTLM relay / exfiltración de hash | Rechazo de UNC y unidades de red (1, 7) |
| ADS ocultos | Rechazo de `:` fuera de la letra de unidad (1) |
| Dispositivos (`\\.\PhysicalDrive0`, `CON`) | (1) |
| Short names 8.3 (`PROGRA~1`) | La ruta final (5) es la canónica larga |
| Archivos ocultos/sistema | Excluidos salvo `include_hidden = true` |

## `list_directory` — R1

| Aspecto | Valor |
|---|---|
| Permiso | `filesystem.list` |
| Input | `{ "path": string, "limit": int 1..500 = 100 }` |
| Output | `{ "path", "entries": [ { "name", "kind": "file" \| "dir", "size"?, "modifiedAt" } ], "truncated" }` |
| Recursos | `FS:<ruta canónica>` (lectura, compartido) |
| Timeouts | default 3 s, max 10 s |
| Sensibilidad | `output` (nombres de archivo pueden ser sensibles) |

No recursivo en v1 (evita recorridos costosos y exfiltración masiva). `limit` se acota además por `max_list_entries`.

**Dónde falla cada comprobación** (implementado): los pasos 1–3 son textuales y ocurren en `validate`, así que una ruta hostil o fuera de las raíces termina en `task.rejected INVALID_PAYLOAD` sin tocar el disco. Los pasos 4–7 (reparse point, ruta final fuera de las raíces, unidad no fija) solo se pueden saber abriendo el handle: terminan en `task.failed PATH_NOT_ALLOWED`. Las raíces se releen al ejecutar, de modo que un `policy reload` intermedio se aplica antes de actuar.

## `read_file` — R1

| Aspecto | Valor |
|---|---|
| Permiso | `filesystem.read` |
| Input | `{ "path": string, "offset": int ≥ 0 = 0, "maxBytes": int 1..24576 = 16384 }` |
| Output | `{ "path", "encoding": "utf-8", "content", "size", "sha256", "truncated" }` |
| Timeouts | default 3 s, max 10 s |
| Sensibilidad | `input_and_output`: el contenido **nunca** se loguea y se redacta en BD tras *settled* (se conserva hash y tamaño) |

- Solo texto UTF-8 (o UTF-16 con BOM, convertido). Binario (bytes NUL en los primeros 8 KiB) → `BINARY_FILE`.
- Archivos > `max_read_bytes` → se permite leer por ventanas con `offset`, nunca más de 24 KiB por tarea.
- Extensiones denegadas por defecto aunque estén en una raíz permitida: `.pfx`, `.p12`, `.pem`, `.key`, `.kdbx`, `.ppk`, `.rdp`, `.ovpn`, `id_*` sin extensión.

## Futuro: escritura

`write_file`/`move_file` (R3, aprobación), `delete_file` (R4, solo local). Requisitos adicionales: escritura atómica (archivo temporal + `MoveFileExW` con `MOVEFILE_REPLACE_EXISTING`), verificación por hash, papelera en lugar de borrado definitivo.
