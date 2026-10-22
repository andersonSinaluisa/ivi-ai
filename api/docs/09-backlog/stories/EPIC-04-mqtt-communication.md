# EPIC-04 — MQTT Communication

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

---

## HU-020 — Configuración de Mosquitto, bootstrap dynsec y ACL

- **Objetivo:** broker configurado según [MQTT Protocol §8-9](../../03-protocols/mqtt-protocol.md#8-autenticación-autorización-y-acl) y [Networking](../../06-infrastructure/networking.md#listeners-de-mosquitto).
- **Descripción:** `mosquitto.conf` (listeners 8883 TLS y 1883 interno, `allow_anonymous false`, `retain_available false`, `max_packet_size 65536`, `max_inflight_messages 20`, `max_queued_messages 1000`, `max_connections 500` en 8883, `persistence true`, plugin dynsec); runbook/script de bootstrap de dynsec ([Secrets Management](../../05-security/secrets-management.md#bootstrap-de-mosquitto-dynsec)) con roles `jarvis-device` y `jarvis-cloud`, client `jarvis-cloud`, ACL por defecto deny.
- **Criterios de aceptación:**
  1. Conexión anónima rechazada.
  2. Un device solo puede publicar en `jarvis/v1/devices/{self}/up/#` y suscribirse a `jarvis/v1/devices/{self}/down/#`: **test que intenta publicar/suscribirse a topics de otro device y verifica que falla o no recibe nada**.
  3. Un device no puede publicar en `down/#` ni en `$CONTROL/#`.
  4. Mensaje retained rechazado/no retenido.
  5. Paquete > 64 KiB → desconexión.
  6. Conexión con `clientId` distinto de `username` → rechazada.
  7. Si `%u` no funciona en la versión fijada, implementar el fallback de rol por device y documentarlo en el ADR-003 (sección Consequences).
- **Dependencias:** HU-007.
- **Consideraciones técnicas:** fijar versión exacta de `eclipse-mosquitto` 2.0.x. En tests, TLS con certificado autofirmado generado al vuelo.
- **Pruebas esperadas:** suite de integración de ACL con dos devices y MQTTnet.
- **Definition of Done:** DoD común + config de Mosquitto versionada en el repo (sin secretos).

---

## HU-021 — Cliente MQTT del Cloud

- **Objetivo:** conexión MQTT confiable de la API con el broker.
- **Descripción:** `MqttCloudClient` (MQTTnet) como hosted service: MQTT 5, `clientId = jarvis-cloud-{instanceId}`, clean start `false`, session expiry 24 h, suscripción `jarvis/v1/devices/+/up/#` QoS 1, **acknowledgement manual** (PUBACK tras procesar), reconexión con backoff (1 s → 30 s), health check `Degraded` si desconectado, métrica `jarvis_mqtt_connected`.
- **Criterios de aceptación:**
  1. Si la API se detiene y un device publica, al volver la API recibe el mensaje (sesión persistente).
  2. Si el handler lanza excepción, el mensaje no se confirma y se reentrega (hasta 3 veces; después se confirma y se registra `Error` para evitar bucles).
  3. Reinicio de Mosquitto → reconexión automática y resuscripción.
  4. Readiness `Degraded` mientras está desconectado.
- **Dependencias:** HU-020, HU-047.
- **Consideraciones técnicas:** el procesamiento por mensaje ocurre en un scope DI nuevo; concurrencia limitada (p. ej. 4 mensajes en paralelo, pero mensajes del mismo device en orden: partición por `deviceId`).
- **Pruebas esperadas:** integración con Mosquitto real, incluyendo reinicio del contenedor.
- **Definition of Done:** DoD común.

---

## HU-022 — Validación de envelope y router de entrada

- **Objetivo:** convertir mensajes `up` en llamadas a casos de uso, con validación de seguridad.
- **Descripción:** parsear envelope ([MQTT Protocol §3](../../03-protocols/mqtt-protocol.md#3-envelope)); aplicar las 7 reglas de validación; comprobar `deviceId` del topic == envelope; comprobar device `Active` (mensajes de devices no activos se descartan); actualizar `last_seen_at` (delegado a presence); enrutar por `(topic suffix, type)` a handlers registrados; scopes de logging `DeviceId`, `MqttMessageId`, `MqttMessageType`, `CorrelationId`.
- **Criterios de aceptación:**
  1. JSON inválido, `v ≠ 1` o campos faltantes → descartado con `Warning`, sin excepción.
  2. `deviceId` distinto al topic → `SECURITY_VIOLATION` (`topic_device_mismatch`).
  3. `type` desconocido → `Information` y descartado.
  4. `ts` fuera de ±5 min en `up/requests` → `request.rejected` `STALE_MESSAGE`.
  5. Campos desconocidos no rompen el parseo.
  6. Métrica `jarvis_mqtt_messages_received_total` con `result`.
- **Dependencias:** HU-021.
- **Consideraciones técnicas:** tipos de `Jarvis.Contracts`; `System.Text.Json` con source generation.
- **Pruebas esperadas:** unit de validación con casos tabulados; integración del router.
- **Definition of Done:** DoD común.

---

## HU-023 — Publicador de mensajes `down`

- **Objetivo:** `IDeviceMessagePublisher` para enviar mensajes a devices.
- **Descripción:** construir envelopes (`id` UUIDv7, `ts`, `deviceId` destino, `correlationId`, `causationId`) y publicar QoS 1 no retained en `down/tasks`, `down/notifications` o `down/control` según tipo; métrica `jarvis_mqtt_messages_published_total`.
- **Criterios de aceptación:**
  1. Cada tipo de mensaje va a su topic correcto (tabla de [MQTT Protocol §2](../../03-protocols/mqtt-protocol.md#2-topics)).
  2. Publicar con el broker caído devuelve error sin lanzar hacia el caso de uso que ya hizo commit (se registra y los mecanismos de redelivery lo cubren).
  3. Nunca se publica con retain.
- **Dependencias:** HU-021.
- **Consideraciones técnicas:** llamado **después** del commit ([Persistence Strategy](../../04-data/persistence-strategy.md#transacción--publicación-sin-outbox-dedicado)).
- **Pruebas esperadas:** integración: un suscriptor de prueba con credencial de device recibe el mensaje en su topic.
- **Definition of Done:** DoD común.
