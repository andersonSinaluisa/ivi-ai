# EPIC-09 — Security

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [Security Architecture](../../05-security/security-architecture.md), [Authorization](../../05-security/authorization.md), [Threat Model](../../05-security/threat-model.md).

---

## HU-043 — Principal actual, políticas y filtro por owner

- **Objetivo:** autorización transversal uniforme (mitiga T06).
- **Descripción:** `ICurrentPrincipal` (tipo, id, ownerId) desde claims, validado contra estado en BD con caché 30 s; políticas `UserOnly`, `DeviceOnly`, `UserOrDevice`; global query filter de EF Core por `owner_id` en `devices`, `tasks`, `device_task_grants`, `events`; `SystemScope` explícito para background services.
- **Criterios de aceptación:**
  1. Test parametrizado que recorre **todos** los endpoints con un token de otro owner y espera 404/403 según [la matriz](../../05-security/authorization.md#matriz-rest).
  2. Una consulta sin principal ni `SystemScope` falla en lugar de devolver datos de todos los owners.
  3. User deshabilitado o device deshabilitado/revocado → rechazado en ≤ 30 s aunque el token no haya expirado.
- **Dependencias:** HU-009.
- **Consideraciones técnicas:** el test del criterio 1 se amplía automáticamente al agregar endpoints (descubrimiento por `EndpointDataSource`).
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-044 — Rate limiting y lockout

- **Objetivo:** limitar abuso a nivel de aplicación ([Security Architecture — Rate limiting](../../05-security/security-architecture.md#rate-limiting)).
- **Descripción:** `System.Threading.RateLimiting`: políticas por principal para `POST /tasks` (30/min), por IP para `/auth/*`; limitador por device para mensajes MQTT (20/s) y `task.create` (30/min); `429` con `Retry-After` y Problem Details `RATE_LIMITED`; `SECURITY_VIOLATION message_flood` si el exceso MQTT persiste 60 s.
- **Criterios de aceptación:**
  1. Exceder cada límite produce la respuesta documentada.
  2. La IP real se toma de `X-Forwarded-For` solo si viene de la red de nginx.
  3. Métrica de rechazos.
- **Dependencias:** HU-009, HU-017.
- **Consideraciones técnicas:** particiones en memoria con expiración para no crecer sin límite.
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-045 — Hardening HTTP de la API

- **Objetivo:** configuración segura del host ASP.NET Core.
- **Descripción:** `ForwardedHeaders` con `KnownNetworks` = subred `edge`; CORS deshabilitado (lista de orígenes configurable vacía); sin Swagger UI en `Production` (OpenAPI generado en build); límites de Kestrel (body 64 KiB, headers); `Server` header eliminado; `/metrics` solo en el puerto interno 8081.
- **Criterios de aceptación:**
  1. `X-Forwarded-For` desde una IP no confiable se ignora.
  2. Request de otro origen no recibe headers CORS.
  3. `/swagger` → 404 en producción.
  4. Body > 64 KiB → 413.
  5. `/metrics` no responde en 8080.
- **Dependencias:** HU-004.
- **Consideraciones técnicas:** headers de seguridad de respuesta los pone nginx ([Nginx](../../06-infrastructure/nginx.md#headers-de-respuesta)); la API no los duplica.
- **Pruebas esperadas:** integración.
- **Definition of Done:** DoD común.

---

## HU-046 — Verificación de redacción de datos sensibles

- **Objetivo:** mitigar T12 con pruebas automáticas.
- **Descripción:** suite que captura toda la salida de logs y eventos durante: bootstrap-owner, login, refresh, registro de device, rotación, device-token, creación de tarea con payload marcador, resultado con marcador; y verifica que ninguno de los valores sensibles aparece.
- **Criterios de aceptación:**
  1. Contraseña, secreto de device, JWT, refresh token, contenido de `payload` y `result` (marcadores únicos) no aparecen en logs ni en `events.data`.
  2. La suite corre en CI.
- **Dependencias:** HU-012, HU-026.
- **Consideraciones técnicas:** logger provider en memoria + consulta a la tabla `events`.
- **Pruebas esperadas:** la propia suite.
- **Definition of Done:** DoD común.
