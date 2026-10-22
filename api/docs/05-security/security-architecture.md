# Security Architecture

## Premisa

Jarvis terminará ejecutando acciones en la PC del owner. Un compromiso del Cloud no debe equivaler a ejecución remota arbitraria. Por eso la seguridad se diseña en capas donde **ninguna capa sola** es suficiente para causar daño:

```text
Transporte (TLS) → Identidad (credencial por device / user) → Autorización Cloud (owner scoping + grants + ACL MQTT)
→ Validación Cloud (formato, tamaño, expiración) → Autorización Device (allowlist + Permission Engine + aprobación local)
→ (Phase 3) Firma end-to-end del principal creador
```

Documentos relacionados: [Trust Boundaries](../01-architecture/trust-boundaries.md) · [Threat Model](threat-model.md) · [Device Authentication](device-authentication.md) · [Authorization](authorization.md) · [Secrets Management](secrets-management.md).

## Controles por principio

| Principio | Control concreto |
|---|---|
| Least privilege | Rol BD `jarvis_app` sin DDL ni `DELETE` en `events`; ACL MQTT por device; grants explícitos por `(source, target, taskType)`; contenedores no-root, `read_only`, `cap_drop: ALL`; solo 3 puertos públicos. |
| Zero trust entre devices | Identidad y credencial individual; los devices no se ven entre sí; un device solo publica en `…/{self}/up/#`; el Cloud valida que el emisor sea el target de la tarea. |
| Autenticación por device | Secreto de 256 bits, único por device. Ver [Device Authentication](device-authentication.md). |
| Autorización | Owner scoping en todas las consultas; políticas User vs Device. Ver [Authorization](authorization.md). |
| Secretos seguros | Docker secrets en archivo (`/run/secrets`), nunca en variables de entorno ni en la imagen ni en git. Ver [Secrets Management](secrets-management.md). |
| TLS | HTTPS (nginx) y MQTT TLS (Mosquitto) con Let's Encrypt; TLS 1.2 mínimo, 1.3 preferido; HSTS. |
| Rate limiting | Perimetral (nginx) + aplicativo (ASP.NET Core RateLimiter) + broker (límites de conexión/paquete). |
| Auditoría | Event log append-only en la misma transacción que el cambio. |
| Revocación | Revoke de device elimina el client dynsec (desconexión inmediata) y hace inválidos sus JWT en ≤ 30 s (chequeo de estado cacheado). Logout y reuse-detection en refresh tokens. |
| Anti-replay | TLS en todo el tráfico; `ts` ±5 min en `up/requests`; `idempotencyKey` obligatoria para devices; `expiresAt` + dedup por `taskId` en el device; `jti` + expiración corta en JWT. |
| Separación transporte / ejecución | El Cloud no ejecuta nada; no hay endpoint de ejecución genérica; `TaskType` y `payload` son opacos; el agente decide. |

## Autenticación de users

- Email + contraseña. Hash con `PasswordHasher<T>` de ASP.NET Core Identity (PBKDF2-HMAC-SHA512, 100 000 iteraciones, sal por usuario). Evaluado Argon2id: más resistente a GPU, pero requiere dependencia nativa; PBKDF2 con este costo es aceptable para un único owner con contraseña fuerte. Revisar al abrir registro de más usuarios.
- Política de contraseña: mínimo 12 caracteres, sin reglas de composición; se rechazan contraseñas de una lista local de contraseñas comunes (`Application/Auth/common-passwords.txt`; la versión inicial es reducida y debe ampliarse hacia el top 10 000).
- Lockout: 5 fallos consecutivos → bloqueo 15 min (`423 ACCOUNT_LOCKED`).
- Access token JWT 15 min; refresh token opaco 30 días, rotación en cada uso, detección de reutilización (revoca la familia).
- Sin registro público: el primer owner se crea con `jarvis-api bootstrap-owner` dentro del contenedor.
- MFA (TOTP): post-MVP, recomendado antes de exponer la UI de Phase 3.

## JWT

| Tema | Decisión |
|---|---|
| Algoritmo | HS256 con clave de 256 bits desde `/run/secrets/jwt_signing_key`. Emisor y validador son el mismo proceso; asimétrico (ES256) cuando otro servicio necesite validar. |
| Claims | `sub` (userId o deviceId), `typ` (`user`\|`device`), `owner_id`, `jti`, `iat`, `exp`, `iss = jarvis-cloud`, `aud = jarvis-api` |
| Vida | User 15 min, Device 10 min |
| Validación | `iss`, `aud`, `exp` con skew de 30 s, `typ` coherente con el endpoint |
| Revocación | Estado del principal (user `Active`, device `Active`) verificado en cada request con caché en memoria de 30 s |
| Rotación de clave | Soporta dos claves (`current`, `previous`) durante 1 h para rotar sin cortar sesiones |

## Rate limiting

| Capa | Regla | Valor |
|---|---|---|
| nginx | `/api/v1/auth/*` por IP | 10 req/min, burst 5 |
| nginx | `/api/*` por IP | 20 req/s, burst 40 |
| nginx | `/hubs/*` conexiones simultáneas por IP | 10 |
| API | `/api/v1/auth/*` por IP | 20/min (configurable: `Jarvis:Security:AuthRequestsPerMinute`) |
| API | Login por email | 5 intentos/15 min (lockout) |
| API | `POST /api/v1/tasks` por principal | 30/min |
| API | `task.create` MQTT por device | 30/min |
| API | Mensajes `up` por device | 20/s sostenido (exceso se descarta con log `Warning`; si persiste 60 s, evento `SECURITY_VIOLATION` `message_flood`) |
| Mosquitto | Conexiones listener 8883 | 500 |
| Mosquitto | Tamaño de paquete | 64 KiB |

Implementación aplicativa: `System.Threading.RateLimiting` en memoria (una instancia). Ver [ADR-008](../08-decisions/ADR-008-redis-decision.md).

## Headers y endurecimiento HTTP

Aplicados en nginx ([Nginx](../06-infrastructure/nginx.md)): HSTS (`max-age=31536000; includeSubDomains`), `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'` (la API no sirve HTML), `server_tokens off`.

En la API: CORS deshabilitado por defecto; lista explícita de orígenes cuando exista UI web. `ForwardedHeaders` solo desde la subred de nginx. Sin Swagger UI en producción (el documento OpenAPI puede generarse en build).

## Logging seguro

Nunca se registran: contraseñas, secretos de device, JWT, refresh tokens, header `Authorization`, query string de `/hubs/*`, `payload` ni `result` de tareas (solo tamaño), cuerpos de requests de `/auth/*`. Detalle en [Observability](../06-infrastructure/observability.md).

## Firma end-to-end de tareas (implementado, [ADR-012](../08-decisions/ADR-012-end-to-end-task-signing.md))

Riesgo que resuelve: un atacante con control del VPS puede crear tareas válidas para el Windows Agent.

1. El cliente que origina la petición (Owner Console, móvil, CLI, un device) tiene un par Ed25519; **la clave privada nunca sale de él y el Cloud no la ve jamás**.
2. El agente Windows guarda las claves públicas en las que confía en su `policy.toml`, instaladas fuera del Cloud. Ese es el trust anchor.
3. El cliente firma una cadena canónica con `taskId`, `type`, el hash del payload, `requestedBy` y `expiresAt` ([MQTT §12](../03-protocols/mqtt-protocol.md#12-firma-end-to-end-adr-012)); la firma viaja en `task.dispatch.data.signature`.
4. El agente exige firma válida por encima del riesgo que fije su política, y rechaza con `SIGNATURE_INVALID` cualquier firma que no cuadre —incluso por debajo de ese umbral—.
5. Lo mismo aplica a las decisiones de aprobación remotas, con `nonce` de un solo uso y ventana de frescura.

**El Cloud no firma, no verifica y no guarda claves.** Comprueba la forma (`alg`, `keyId`, 64 bytes en base64) y retransmite. Es deliberado: si el Cloud pudiera producir firmas válidas, comprometerlo seguiría bastando para inyectar tareas, que es justo lo que esto evita.

Lo que un Cloud comprometido **todavía** puede hacer: omitir tareas, retrasarlas, duplicarlas o crear tareas *sin* firma. Lo último lo corta la política del agente (`[signatures] require_above`).

## Riesgos aceptados en el MVP

| Riesgo | Motivo de aceptación | Revisión |
|---|---|---|
| Un VPS comprometido puede inyectar tareas **sin firma** | Mitigado end-to-end para las firmadas (ADR-012); las no firmadas dependen de que el owner suba `require_above` en la política del agente | Al publicar la Owner Console, que es quien debería exigirlo por defecto |
| Sin MFA | Un solo owner, contraseña fuerte, lockout | Antes de UI pública (Phase 3) |
| Tráfico interno Docker sin TLS (API ↔ Mosquitto, API ↔ PostgreSQL) | Red bridge interna de un único host | Si se separan hosts |
| Clave JWT simétrica | Emisor = validador | Al agregar servicios |
