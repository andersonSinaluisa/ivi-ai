# ADR-016: MQTT 3.1.1 y excepción de texto plano en loopback

## Status

Accepted, 2026-09-19. Modifica detalles de [MQTT Client](../02-cloud-integration/mqtt-client.md) tras implementar el Cloud Link.

## Context

Dos decisiones previas chocaron con la realidad al implementar:

1. **Versión del protocolo.** La documentación pedía MQTT 5 con fallback a 3.1.1. El cliente elegido (`rumqttc`) tiene dos APIs: la estable (3.1.1) y el módulo `v5`, menos maduro. El protocolo de Jarvis **no usa ninguna característica de MQTT 5** (ni propiedades, ni razones de DISCONNECT, ni suscripciones compartidas del lado device): el envelope JSON lleva toda la semántica. El Cloud acepta ambas versiones ([MQTT Protocol §5](../../../api/docs/03-protocols/mqtt-protocol.md#5-conexión)).
2. **TLS obligatorio.** La documentación decía "sin opción de desactivarla en builds release". El entorno de desarrollo del Cloud (`api/deploy/docker-compose.dev.yml`) expone Mosquitto **sin TLS** en `127.0.0.1:1883`, así que con la regla literal el agente no podía probarse contra el Cloud real sin generar certificados para cada máquina de desarrollo.

## Decision

1. El agente habla **MQTT 3.1.1**. Si en el futuro el protocolo necesita algo de MQTT 5 (p. ej. *request/response* o expiración de mensajes), se revisa esta ADR.
2. **TLS es obligatorio salvo para hosts loopback.** `cloud.broker_url` acepta:
   - `mqtts://host:8883` — TLS 1.2+ con validación de cadena y hostname contra el almacén de certificados de Windows (`rustls-platform-verifier`); `cloud.ca_file` permite una CA propia.
   - `mqtt://127.0.0.1:1883`, `mqtt://localhost:1883`, `mqtt://[::1]:1883` — texto plano **solo** si el host es loopback.
   Cualquier `mqtt://` con host remoto se rechaza al validar la configuración, en todas las builds. La comprobación está en `BrokerUrl::parse` y cubierta por tests.

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Módulo `v5` de `rumqttc` | Menos maduro y sin beneficio: el protocolo no usa nada de v5 |
| Flag global `--insecure` | Un flag así acaba activado en producción "temporalmente"; la restricción a loopback no se puede usar contra un broker remoto |
| Generar certificados de desarrollo por máquina | Fricción alta para cada desarrollador; el tráfico loopback no sale de la máquina |
| Compilar la excepción solo en builds de debug | El binario de desarrollo dejaría de ser el que se prueba; la regla por host es más simple de auditar |

## Consequences

- ✅ El agente se prueba contra el Cloud de desarrollo real sin bajar la seguridad de producción: fuera de loopback no hay forma de desactivar TLS.
- ✅ Menos superficie de bugs en el cliente MQTT.
- ⚠️ El tráfico loopback en texto plano es visible para otros procesos del mismo equipo con permisos de captura; aceptable porque en ese escenario el broker es local y de desarrollo.
- ⚠️ Si el Cloud dejara de aceptar 3.1.1, habría que migrar al módulo v5.
