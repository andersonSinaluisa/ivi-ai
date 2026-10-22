# ADR-009: Credenciales en Windows Credential Manager

## Status

Accepted, 2026-09-19.

## Context

El agente necesita el device secret para autenticarse en Mosquitto. Los archivos de configuración se comparten, se copian a backups y se pegan en tickets; `.env` y variables de entorno se filtran a procesos hijo y volcados.

## Decision

- El secreto vive en **Windows Credential Manager** (`CRED_TYPE_GENERIC`, target `Jarvis/WindowsAgent/<deviceId>`, `CRED_PERSIST_LOCAL_MACHINE`), protegido por DPAPI del usuario.
- Nunca en `config.toml`, `config.json`, `.env`, variables de entorno, registro, SQLite ni logs. El agente se niega a arrancar si detecta claves sospechosas en la configuración.
- Provisión por consola sin eco o stdin, nunca por argumento.
- Tipos `Secret<T>` con `zeroize` y `Debug` redactado.

Detalle en [Credential Storage](../04-security/credential-storage.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Archivo cifrado con DPAPI propio | Equivalente, más código propio |
| Certificado de cliente (mTLS) con clave no exportable / TPM | Más fuerte; el Cloud v1 usa usuario/contraseña. Evolución futura |
| Texto plano en config / `.env` | Prohibido por requisito |

## Consequences

- ✅ Secreto protegido frente a otros usuarios y fuera de los archivos compartibles.
- ⚠️ Malware del mismo usuario puede leerlo (riesgo residual documentado; impacto acotado por ACL del broker).
- ⚠️ Mover el agente a otra PC exige re-provisionar (deseable).
