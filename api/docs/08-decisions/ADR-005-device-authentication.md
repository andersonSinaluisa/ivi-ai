# ADR-005 — Device authentication

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Cada device necesita identidad propia, revocable, usable desde ESP32 (mbedTLS, memoria limitada) y desde Windows/mobile, para MQTT y opcionalmente REST. No se deben almacenar secretos en texto plano.

## Decision

- **Secreto por device** de 256 bits, formato `jdv_<base64url>`, mostrado una sola vez al registrar o rotar.
- PostgreSQL guarda `HMAC-SHA256(pepper, secret)`; el pepper vive en Docker secrets.
- MQTT: `username = clientId = deviceId`, `password = secret`, verificado por Mosquitto dynsec.
- REST: `POST /api/v1/auth/device-token` intercambia el secreto por un JWT de 10 min.
- Registro solo por un owner autenticado (sin auto-registro anónimo).
- Una credencial activa por device; rotación sin gracia; revocación inmediata.
- Modelo preparado para `ClientCertificate` y expiración.

Detalle: [Device Authentication](../05-security/device-authentication.md).

## Alternatives

| Alternativa | Ventajas | Desventajas |
|---|---|---|
| **mTLS con certificados de cliente desde el MVP** | Clave privada no exportable, rotación con solapamiento, estándar | Requiere CA propia, emisión, CRL/revocación en Mosquitto (poco dinámica), provisión más compleja en ESP32; demasiado para MVP |
| JWT de larga duración como password MQTT | Sin estado en el broker | Revocación difícil; requiere auth plugin |
| Contraseña elegida por el owner | Simple | Baja entropía; reutilización |
| Hash lento (Argon2id) del secreto | Estándar para contraseñas | Innecesario para 256 bits de entropía; costo de CPU por request |
| API keys compartidas por tipo de device | Simple | Viola zero trust: un robo compromete a todos |

## Consequences

- (+) Simple de implementar en ESP-IDF (usuario/contraseña MQTT sobre TLS).
- (+) Revocación inmediata en ambos canales.
- (−) El secreto es "portador": quien lo copie puede usarlo. Mitigado con grants mínimos, NVS cifrado y revocación.
- (−) Rotación requiere reprovisionar el device (sin gracia en MVP).
- Evolución: mTLS en Phase 6 para dispositivos físicos si el riesgo lo justifica.
