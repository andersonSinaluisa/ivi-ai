# ADR-003 — MQTT for device communication

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Los devices necesitan recibir tareas en tiempo real (push), mantener presencia, funcionar en redes NAT, y el ESP32 tiene recursos limitados. El Cloud necesita autenticación y ACL por device, y detectar desconexiones.

## Decision

1. **MQTT** como protocolo device ↔ Cloud (3.1.1 compatible, 5.0 preferido; el protocolo de aplicación no depende de features de v5).
2. **Eclipse Mosquitto 2.x** como broker, con el plugin **Dynamic Security** administrado exclusivamente por la API vía `$CONTROL`.
3. Topics por dirección `jarvis/v1/devices/{deviceId}/up|down/...` y envelope JSON versionado ([MQTT Protocol](../03-protocols/mqtt-protocol.md)).
4. TLS terminado en Mosquitto en 8883; la API usa un listener interno 1883.

## Alternatives

| Alternativa | Ventajas | Desventajas |
|---|---|---|
| **Broker embebido en la API (MQTTnet Server)** | Un proceso menos; auth y ACL directamente contra PostgreSQL; sin sincronizar credenciales | Cada despliegue de la API desconecta todos los devices; exposición directa del proceso de negocio a Internet por 8883; menor madurez frente a Mosquitto como servidor público |
| **Mosquitto + auth plugin HTTP** (p. ej. mosquitto-go-auth contra la API) | Fuente única de credenciales en PostgreSQL | Plugin de terceros a compilar/mantener; la autenticación de devices depende de que la API esté arriba |
| **Mosquitto + password file estático** | Simple | Requiere recargar el broker en cada alta/baja; no escalable ni revocable en caliente |
| **EMQX** | Auth/ACL por HTTP o SQL integradas, dashboard | Consumo de memoria mucho mayor (JVM-like/Erlang, cientos de MB); más superficie |
| **NATS** | Rápido, ligero | Cliente ESP32 menos maduro; sin LWT nativo equivalente |
| **WebSocket propio** | Control total | Reimplementar keepalive, QoS, reconexión, LWT; peor en ESP32 |
| **HTTP polling** | Trivial | Latencia, batería/consumo, carga en el VPS |

## Consequences

- (+) Estándar con soporte nativo en ESP-IDF, .NET, Rust, JS.
- (+) LWT y keepalive resuelven detección de desconexión.
- (+) Revocación inmediata (`deleteClient`/`disableClient` desconectan).
- (−) Credenciales duplicadas en dynsec (derivadas de PostgreSQL) → reconciliación y backup del volumen de Mosquitto.
- Implementación (2026-09-19): se adoptó directamente el fallback de **un rol dynsec por device con topics literales** (`jarvis-device-{deviceId}`), que no depende de la sustitución `%u`. El aislamiento entre devices está cubierto por `BrokerAclTests`.
- (−) Puerto 8883 puede estar bloqueado en algunas redes → plan B MQTT sobre WSS por 443.
