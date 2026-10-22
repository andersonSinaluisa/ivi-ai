# MQTT Client

Implementación del lado device de [MQTT Protocol v1 §5](../../../api/docs/03-protocols/mqtt-protocol.md#5-conexión). Biblioteca: `rumqttc` (módulo `v5`), TLS con `rustls` y verificación contra el almacén de certificados de Windows.

## Parámetros de conexión

| Parámetro | Valor | Origen |
|---|---|---|
| Host / puerto | `cloud.broker_url` de `config.toml` (`mqtts://host:8883`) | Devueltos por el Cloud al registrar el device |
| Versión | **MQTT 3.1.1** (el protocolo no usa nada de v5; [ADR-016](../09-decisions/ADR-016-mqtt-transport-and-loopback.md)) | Cloud |
| `clientId` / `username` | `deviceId` | Cloud (dynsec fija el clientId) |
| `password` | Secreto del device desde Credential Manager, leído al conectar y descartado de memoria tras el `CONNECT` | [Credential Storage](../04-security/credential-storage.md) |
| Clean start | `true` | El Cloud redespacha desde PostgreSQL; evita comandos viejos encolados |
| Keepalive | 60 s | Cloud |
| LWT | Topic `up/presence`, `presence.offline { sessionId }`, QoS 1, **retain false** | Cloud |
| TLS | TLS 1.2+, verificación de cadena y hostname obligatoria contra el almacén de certificados de Windows. Única excepción: `mqtt://` **solo** si el host es loopback (broker de desarrollo en la propia máquina); con host remoto la configuración se rechaza en todas las builds ([ADR-016](../09-decisions/ADR-016-mqtt-transport-and-loopback.md)). `cloud.ca_file` opcional para CA propia | Cloud Threat Model T09 |
| Tamaño máx. de paquete | 64 KiB | Cloud |

## Secuencia de conexión

1. Generar `sessionId` (≥ 64 bits aleatorios) y construir el LWT con él.
2. `CONNECT`; si `CONNACK` = *Bad username or password* / *Not authorized* → estado `AUTH_REJECTED` ([Reconnection](reconnection.md#autenticación-rechazada)).
3. `SUBSCRIBE jarvis/v1/devices/{self}/down/#` QoS 1.
4. Publicar `presence.hello` (ver [Presence](presence.md)) con `openTaskIds` del State Store.
5. Esperar `presence.hello.ack` con el mismo `sessionId` (timeout 10 s → desconectar y reintentar).
6. Estado **Ready**: calcular desfase de reloj; vaciar el outbox en orden (reenvío del último estado de cada tarea no *settled*); iniciar heartbeats.

## Recepción

- Solo se procesan topics `jarvis/v1/devices/{self}/down/{tasks|control|notifications}`; cualquier otro se descarta con `WARN`.
- El envelope se valida (`v == 1`, `deviceId == self`, `id`, `type`, `ts`); campos desconocidos se ignoran (compatibilidad hacia adelante del Cloud).
- `task.*` se entregan al Task Pipeline por un canal acotado (capacidad 64). Si el canal está lleno, el mensaje se descarta **sin** ack de aplicación: el Cloud lo redespachará tras su ACK timeout. Nunca se bloquea el event loop MQTT.
- El PUBACK MQTT lo envía la biblioteca al recibir; el **ACK de aplicación** es `task.ack`, que solo se publica tras persistir (W5). Una pérdida entre PUBACK y persistencia la cubre el ACK timeout del Cloud.

## Publicación

- Todo mensaje `up/tasks` sale **del outbox** (ya persistido) y se publica con QoS 1, retain false.
- Si no hay conexión, los mensajes esperan en el outbox; no hay cola en memoria que se pueda perder.
- `presence.heartbeat` es QoS 0 y no se persiste.

## Duplicados

| Duplicado | Tratamiento |
|---|---|
| Mismo `task.dispatch` (mismo `taskId`, cualquier `attempt`) | Reenviar último estado; no ejecutar ([Idempotency](../06-state/idempotency.md)) |
| `task.cancel` repetido | Idempotente |
| `task.update.ack` repetido o para tarea desconocida | Ignorar |
| `presence.hello.ack` de otra sesión | Ignorar |

## Versionado

El agente declara `protocolVersion: "1.0"` y, cuando envíe `sessionState` ([C11](../00-overview/cloud-compatibility.md)), `"1.1"`. Solo se añaden campos opcionales; nunca se cambia la semántica de v1.
