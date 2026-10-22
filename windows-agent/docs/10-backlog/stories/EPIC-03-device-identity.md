# EPIC-03 — Device Identity

Fase **P2**. Objetivo: el agente se identifica ante el Cloud con una credencial que nunca toca archivos ni logs. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-012 — Adaptador de Credential Manager

**Goal**: guardar, leer y borrar el device secret de forma segura.

**Description**: interfaz `CredentialStore` en `jarvis-windows` según [Credential Storage](../../04-security/credential-storage.md#credential-manager).

**Acceptance Criteria**
1. `put(deviceId, Secret)`, `get(deviceId) -> Option<Secret>`, `delete(deviceId)`.
2. `CRED_TYPE_GENERIC`, target `Jarvis/WindowsAgent/<deviceId>`, `UserName = deviceId`, `CRED_PERSIST_LOCAL_MACHINE`.
3. `Secret` con `zeroize` al soltarse y `Debug` = `***`.
4. Los buffers devueltos por `CredReadW` se liberan con `CredFree` y se copian a `Secret` sin copias intermedias en `String`.
5. Fake en memoria para tests.

**Dependencies**: HU-WIN-001.

**Technical Considerations**: `CredentialBlob` en UTF-16 o bytes: documentar la codificación elegida (UTF-8 en bytes) y mantenerla.

**Tests**: integración Windows: put/get/delete con un target de test único y limpieza al terminar.

**Definition of Done**: DoD común.

---

## HU-WIN-013 — Comando `provision`

**Goal**: dar de alta el agente con su credencial de forma segura y verificada.

**Description**: `jarvis-agent provision --device-id <id> --host <h> [--port 8883] [--secret-stdin]` según [Credential Storage](../../04-security/credential-storage.md#provisión).

**Acceptance Criteria**
1. El secreto se pide por consola sin eco o se lee de stdin; **nunca** se acepta como argumento (no existe flag `--secret`).
2. Antes de guardar, prueba una conexión MQTT con esas credenciales; si falla por autenticación, no guarda nada y lo explica.
3. Guarda el secreto (HU-WIN-012) y escribe `device.device_id` y `cloud.broker_url` en `config.toml` preservando el resto del archivo.
4. Ofrece crear un `policy.toml` inicial con el owner como principal (pregunta interactiva; `--no-policy` para omitirlo), con capabilities R0–R2 de procesos.
5. Si el agente está corriendo, le notifica por pipe para reconectar.

**Dependencies**: HU-WIN-012, HU-WIN-007, HU-WIN-005, HU-WIN-002.

**Technical Considerations**: `rpassword` o `ReadConsoleW` con `ENABLE_ECHO_INPUT` desactivado; edición de TOML preservando comentarios (`toml_edit`).

**Tests**: integración con broker de prueba: secreto correcto/incorrecto; test de que el proceso no expone el secreto en su línea de comandos.

**Definition of Done**: DoD común.

---

## HU-WIN-014 — Uso de la credencial en el Cloud Link

**Goal**: conectar al Cloud con la credencial provisionada y reaccionar a su ausencia o revocación.

**Description**: integrar `CredentialStore` en el Cloud Link (HU-WIN-007) y cubrir los estados de identidad de [Health](../../07-observability/health.md#condiciones).

**Acceptance Criteria**
1. Sin `device_id` o sin credencial → condición `NOT_PROVISIONED`, sin intentos de conexión, `status` indica `jarvis-agent provision`.
2. La credencial se lee de Credential Manager en cada intento de conexión (no se cachea más allá de la vida del CONNECT), de modo que un `provision` posterior surte efecto.
3. El buffer propio del secreto se sobrescribe tras entregarlo al cliente MQTT.
4. Revocación/rotación (CONNACK rechazado) → `AUTH_REJECTED` con el backoff de HU-WIN-009; recuperación inmediata tras `provision`.
5. Test canario: el secreto no aparece en logs, `status` ni `diagnostics`.

**Dependencies**: HU-WIN-012, HU-WIN-013, HU-WIN-009.

**Technical Considerations**: el trait `CredentialProvider` se inyecta en el Cloud Link (fake en tests portables).

**Tests**: integración con Jarvis Cloud real: device registrado → `Online`; rotar credencial en el Cloud → `AUTH_REJECTED`; `provision` con la nueva → `Online` sin reiniciar el agente.

**Definition of Done**: DoD común + escenario [WIN-E2E-04](../../11-testing/integration-scenarios.md#win-e2e-04-device-revocado) verde.
