# EPIC-03 — Device Authentication

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

---

## HU-017 — Token de device para REST

- **Objetivo:** `POST /api/v1/auth/device-token` según [REST API](../../03-protocols/rest-api.md#post-apiv1authdevice-token).
- **Descripción:** verificar `HMAC(pepper, secret)` en tiempo constante contra la credencial `Active`; emitir JWT `typ=device` de 10 min; actualizar `last_used_at` como máximo una vez por minuto; `GET /api/v1/me` para devices; evento `DEVICE_AUTH_FAILED` en fallos.
- **Criterios de aceptación:**
  1. Secreto correcto → 200 y token válido en `/api/v1/me` (principalType `Device`).
  2. Device inexistente, secreto incorrecto o credencial revocada → mismo 401 `INVALID_CREDENTIALS`.
  3. Device `Disabled` → 403 `DEVICE_DISABLED`; `Revoked` → 401.
  4. Tras disable/revoke, un token de device ya emitido deja de ser aceptado en ≤ 30 s.
  5. Un token de device no accede a endpoints `UserOnly` (403).
- **Dependencias:** HU-012.
- **Consideraciones técnicas:** `CryptographicOperations.FixedTimeEquals`. Caché del estado del principal 30 s ([Security Architecture](../../05-security/security-architecture.md#jwt)).
- **Pruebas esperadas:** integración de todos los criterios con `TimeProvider` falso para el caso de 30 s.
- **Definition of Done:** DoD común.

---

## HU-018 — Adaptador de administración Dynamic Security

- **Objetivo:** implementar `IBrokerCredentialAdmin` sobre `$CONTROL/dynamic-security/v1`.
- **Descripción:** operaciones `CreateClient(deviceId, secret)`, `SetPassword`, `DisableClient`, `EnableClient`, `DeleteClient`, `ListClients`, `GetClient`; correlación de request/response del plugin; timeout 5 s; errores tipados.
- **Criterios de aceptación:**
  1. `CreateClient` crea el client con `clientid = username = deviceId` y rol `jarvis-device`.
  2. Operaciones idempotentes desde el punto de vista del llamador (crear existente → error tipado `AlreadyExists`; borrar inexistente → éxito).
  3. Timeout → excepción tipada que los casos de uso traducen a 503.
  4. El secreto no aparece en logs (ni en nivel `Debug`).
  5. Disable de un client conectado lo desconecta (verificado).
- **Dependencias:** HU-020.
- **Consideraciones técnicas:** usa la misma conexión MQTT del cloud client (HU-021) o una conexión administrativa dedicada del mismo usuario `jarvis-cloud`; decidir en implementación y documentar. Si HU-021 no existe aún, crear la conexión mínima necesaria aquí y reutilizarla después.
- **Pruebas esperadas:** integración contra Mosquitto real (Testcontainers) con la config de HU-020.
- **Definition of Done:** DoD común.

---

## HU-019 — Reconciliación de credenciales con el broker

- **Objetivo:** que dynsec converja al estado de PostgreSQL.
- **Descripción:** `BrokerReconciliationService`: al arrancar, cada 1 h y cada 30 s mientras existan diferencias pendientes. Compara clients dynsec con devices: elimina clients sin device (o revocados), aplica disabled/enabled, verifica rol; si un device activo no existe en dynsec, marca `broker_synced_at = null` y log `Warning` (requiere rotación, ver [Persistence Strategy](../../04-data/persistence-strategy.md#reconciliación-de-dynsec-sin-secretos-en-claro)).
- **Criterios de aceptación:**
  1. Un client huérfano en dynsec se elimina en el siguiente ciclo.
  2. Un device deshabilitado mientras el broker estaba caído queda deshabilitado en dynsec ≤ 30 s después de que el broker vuelve.
  3. Clients del sistema (`jarvis-cloud`) nunca se tocan.
  4. Readiness reporta `Degraded` hasta la primera reconciliación exitosa.
  5. La API expone el estado en `GET /api/v1/devices/{id}` (campo `brokerSynced`, falso si `broker_synced_at` es nulo; ver [REST API](../../03-protocols/rest-api.md#representación-device)).
- **Dependencias:** HU-018.
- **Consideraciones técnicas:** nunca recrea clients de devices (no tiene el secreto).
- **Pruebas esperadas:** integración: borrar/crear clients manualmente en dynsec y verificar convergencia.
- **Definition of Done:** DoD común.
