# Credential Storage

## Regla

Las credenciales **no** se guardan en `config.toml`, `config.json`, `.env`, variables de entorno persistentes, el registro en claro, la base SQLite ni ningún archivo de texto. Se guardan en **Windows Credential Manager** ([ADR-009](../09-decisions/ADR-009-credential-storage.md)).

## Qué secretos existen

| Secreto | Origen | Uso | Almacén |
|---|---|---|---|
| Device secret MQTT | Emitido por el Cloud al registrar el device (se muestra una vez) | `password` del CONNECT MQTT (`username` = `deviceId`) | Credential Manager, `Jarvis/WindowsAgent/<deviceId>` |
| Clave de firma de updates (pública) | Embebida en el binario | Verificar actualizaciones (P8) | Binario (no es secreto) |
| Claves públicas de aprobadores (P8) | Provisión local | Verificar `task.approval_decision` | `policy.toml` (públicas, no secretas) |
| Credenciales de apps o sitios (futuro browser provider) | — | **Fuera de alcance**: el agente no gestiona contraseñas de terceros | — |

## Credential Manager

| Aspecto | Valor |
|---|---|
| API | `CredWriteW` / `CredReadW` / `CredDeleteW` (`windows-rs`) |
| Tipo | `CRED_TYPE_GENERIC` |
| Target | `Jarvis/WindowsAgent/<deviceId>` |
| `UserName` | `deviceId` |
| Persistencia | `CRED_PERSIST_LOCAL_MACHINE` (solo esta máquina, **no** roaming con el perfil; evita que el secreto viaje a otras PCs del dominio) |
| Protección | DPAPI con la clave del usuario: otro usuario de la PC no puede leerlo; un proceso del mismo usuario sí |

## Provisión

```text
jarvis-agent provision --device-id 0192a6f0-2222-7aaa-8bbb-000000000002 --host jarvis.example.com --port 8883
Device secret: ************  (entrada oculta; nunca por argumento de línea de comandos)
```

- El secreto se lee de la consola sin eco o desde stdin (`--secret-stdin`) para automatización. **Nunca** como argumento: las líneas de comandos son visibles para otros procesos y quedan en historiales.
- Se prueba la conexión antes de guardar; si falla la autenticación no se guarda nada.
- `config.toml` solo recibe `device_id` y `broker_url` (construido con `--host`/`--port`).

## Uso en memoria

- El secreto se lee de Credential Manager al conectar, se pasa al cliente MQTT y se sobrescribe en el buffer propio tras el CONNECT (`zeroize`). El cliente MQTT puede conservar una copia para reconexiones; se acepta como riesgo residual (misma cuenta).
- `Debug`/`Display` de los tipos que contienen secretos imprimen `***`.
- Los logs y diagnósticos pasan por un filtro que redacta cualquier campo llamado `password`, `secret`, `token` o `authorization`.

## Rotación y revocación

| Evento | Comportamiento |
|---|---|
| Owner rota el secreto en el Cloud | El broker rechaza el CONNECT → `AUTH_REJECTED` (backoff 5 min, [Reconnection](../02-cloud-integration/reconnection.md#autenticación-rechazada)) → `jarvis-agent provision` con el nuevo secreto |
| Device deshabilitado/revocado | Igual; el agente no borra la credencial por sí solo (podría ser temporal) |
| `jarvis-agent uninstall` | Borra la credencial (`CredDeleteW`) y la Scheduled Task; datos solo con `--purge` |

## Alternativas descartadas

| Alternativa | Motivo |
|---|---|
| Archivo cifrado con DPAPI propio | Equivalente en seguridad pero reinventa Credential Manager y complica la gestión por el usuario |
| Certificado de cliente TLS en el almacén de Windows (clave no exportable) | Mejor protección de la clave, pero el Cloud v1 autentica por usuario/contraseña en Mosquitto. Candidato futuro (mTLS) |
| TPM / Windows Hello para sellar la clave | Requiere mTLS o firma de challenge; futuro |
| Variables de entorno / `.env` | Visibles por procesos hijo, quedan en volcados y en backups en claro |

## Riesgo residual

Malware con la cuenta del usuario puede leer la credencial. Impacto acotado: con ella puede **conectarse como el device** (publicar presencia/resultados falsos), pero no puede enviar tareas al agente (la ACL solo permite a `jarvis-cloud` publicar en `down/#`). El owner revoca el device en el Cloud.
