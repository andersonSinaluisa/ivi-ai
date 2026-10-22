# EPIC-01 — Cloud Infrastructure

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

---

## HU-001 — Esqueleto de la solución

- **Objetivo:** tener la estructura de proyectos y convenciones de build sobre la que se implementa todo lo demás.
- **Descripción:** crear la solución .NET 10 con `src/Jarvis.Domain`, `src/Jarvis.Contracts`, `src/Jarvis.Application`, `src/Jarvis.Infrastructure`, `src/Jarvis.Api`, `tools/Jarvis.Simulators`, `tests/Jarvis.Domain.Tests`, `tests/Jarvis.Application.Tests`, `tests/Jarvis.IntegrationTests`, `tests/Jarvis.E2ETests`, con las referencias permitidas por [Backend Architecture](../../01-architecture/backend-architecture.md).
- **Criterios de aceptación:**
  1. `dotnet build` y `dotnet test` pasan en limpio desde la raíz.
  2. `Directory.Build.props` con `Nullable=enable`, `TreatWarningsAsErrors=true`, `LangVersion` por defecto de .NET 10, analizadores habilitados.
  3. `Directory.Packages.props` (Central Package Management).
  4. Las referencias entre proyectos respetan el diagrama de dependencias; un test de arquitectura falla si `Jarvis.Domain` referencia otro proyecto o si `Jarvis.Application` referencia `Jarvis.Infrastructure`/`Jarvis.Api`.
  5. `Jarvis.Api` arranca y responde 404 en `/`.
  6. `.gitignore` incluye `bin/`, `obj/`, `secrets/`, `.local-secrets/`, `*.env`, `*.pem`, `*.key`.
  7. `.editorconfig` con estilo del proyecto.
- **Dependencias:** ninguna.
- **Consideraciones técnicas:** test de arquitectura con NetArchTest o reflexión simple sobre referencias de ensamblado. No agregar MediatR ni AutoMapper ([ADR-001](../../08-decisions/ADR-001-backend-technology.md)).
- **Pruebas esperadas:** test de arquitectura; smoke test `WebApplicationFactory` que obtiene 404 en `/`.
- **Definition of Done:** DoD común + README raíz actualizado con comandos de build/test.

---

## HU-002 — Configuración tipada y carga de secretos

- **Objetivo:** leer configuración y secretos de forma segura y fallar temprano si falta algo.
- **Descripción:** clases de opciones (`JarvisOptions`, `MqttOptions`, `JwtOptions`, `TaskOptions`, `PresenceOptions`) enlazadas con `IOptions` y validadas al inicio. Soporte de claves `*File` que leen el valor desde un archivo ([Secrets Management](../../05-security/secrets-management.md)).
- **Criterios de aceptación:**
  1. Con un secreto requerido ausente, la app termina al arrancar con un mensaje que nombra la clave pero **no** muestra valores.
  2. `ConnectionStrings__Postgres` admite `Password_File=` (la app sustituye por el contenido del archivo).
  3. Longitudes mínimas validadas: `jwt_signing_key` ≥ 32 bytes, `device_secret_pepper` ≥ 32 bytes.
  4. Defaults de tiempos iguales a [Task State Machine](../../02-domain/task-state-machine.md#tiempos-por-defecto) y presencia (30/75/180 s).
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** `ValidateOnStart()`. Los valores de secretos se guardan en tipos que redefinen `ToString()` como `"***"`.
- **Pruebas esperadas:** unit tests de binding y validación; test que verifica que el mensaje de error no contiene el valor.
- **Definition of Done:** DoD común + tabla de configuración documentada en [Docker Strategy](../../06-infrastructure/docker-strategy.md#variables-de-entorno-sin-secretos) actualizada si cambian nombres.

---

## HU-003 — Persistencia base PostgreSQL y comando `migrate`

- **Objetivo:** `JarvisDbContext` con convenciones y migraciones aplicables de forma controlada.
- **Descripción:** configurar EF Core + Npgsql, esquema `jarvis`, snake_case, UUIDv7 generado en app, `timestamptz` UTC, `xmin` como concurrency token disponible, subcomando `jarvis-api migrate` que usa el rol `jarvis_migrator`. Script SQL de creación de roles ([Database Design](../../04-data/database-design.md#roles-de-base-de-datos)).
- **Criterios de aceptación:**
  1. `jarvis-api migrate` aplica migraciones y termina con código 0; la API normal **no** migra al iniciar.
  2. La API al arrancar verifica que no hay migraciones pendientes; si las hay, readiness es Unhealthy (HU-006).
  3. Script de roles crea `jarvis_migrator`, `jarvis_app`, `jarvis_maintenance`, `jarvis_backup` con los permisos documentados; `jarvis_app` no puede ejecutar DDL.
  4. Migración inicial vacía (solo esquema); las tablas llegan con sus HUs.
- **Dependencias:** HU-002.
- **Consideraciones técnicas:** `EnableRetryOnFailure` para errores transitorios. `IJarvisDbContext` expuesto a Application. Sin lazy loading.
- **Pruebas esperadas:** integración con Testcontainers PostgreSQL: migrate funciona; `jarvis_app` recibe error al intentar `CREATE TABLE`.
- **Definition of Done:** DoD común.

---

## HU-004 — Logging estructurado y correlation id

- **Objetivo:** todos los logs en JSON con correlación de extremo a extremo.
- **Descripción:** formatter JSON de consola; middleware que toma `X-Correlation-Id` (UUID válido) o `X-Request-Id` o genera uno, lo pone en un scope de logging, en `HttpContext` y en la respuesta. Abstracción `ICorrelationContext` usable también desde MQTT y background services.
- **Criterios de aceptación:**
  1. Cada línea de log de un request incluye `CorrelationId` y `TraceId`.
  2. La respuesta incluye `X-Correlation-Id`.
  3. Un `X-Correlation-Id` inválido (no UUID) se ignora y se genera uno nuevo.
  4. Los headers `Authorization` y query strings no se registran por el logging de requests.
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** ver [Observability](../../06-infrastructure/observability.md#logging-estructurado). Usar `[LoggerMessage]` para mensajes frecuentes.
- **Pruebas esperadas:** test de integración que captura logs (logger provider en memoria) y verifica el scope.
- **Definition of Done:** DoD común.

---

## HU-005 — Problem Details y catálogo de errores

- **Objetivo:** respuestas de error uniformes según [Error Model](../../03-protocols/error-model.md).
- **Descripción:** excepción/resultado de aplicación con `code`; mapeo a RFC 9457 con `type = urn:jarvis:problem:{code-kebab}`, `code`, `correlationId`, `traceId`. Validación con diccionario `errors`. Catálogo de códigos como constantes en `Jarvis.Contracts`.
- **Criterios de aceptación:**
  1. Excepción no controlada → 500 con `detail` genérico, sin stack trace; log `Error` con el detalle.
  2. Error de validación → 400 `VALIDATION_FAILED` con `errors`.
  3. Content-Type `application/problem+json`.
  4. 404 de rutas inexistentes también en Problem Details.
- **Dependencias:** HU-001.
- **Consideraciones técnicas:** `AddProblemDetails` + `IExceptionHandler`. No usar excepciones para control de flujo en casos esperados: los handlers devuelven un `Result` con error tipado.
- **Pruebas esperadas:** tests de integración por cada tipo de respuesta de error.
- **Definition of Done:** DoD común.

---

## HU-006 — Health checks live/ready

- **Objetivo:** exponer `/health/live` y `/health/ready` según [Observability](../../06-infrastructure/observability.md#health-checks).
- **Descripción:** live sin dependencias; ready con PostgreSQL y migraciones (Unhealthy), y registros para MQTT, reconciliación y background services (Degraded) que se completan en sus HUs. Subcomando `jarvis-api healthcheck`.
- **Criterios de aceptación:**
  1. `/health/live` → 200 con PostgreSQL caído.
  2. `/health/ready` → 503 con PostgreSQL caído; 200 con todo sano.
  3. La respuesta no incluye mensajes de excepción ni cadenas de conexión.
  4. `jarvis-api healthcheck` sale con 0/1 según ready.
- **Dependencias:** HU-003.
- **Consideraciones técnicas:** los checks `Degraded` de HUs posteriores se registran mediante un mecanismo extensible (p. ej. `IHeartbeatTracker` que los background services actualizan).
- **Pruebas esperadas:** integración con PostgreSQL detenido/arrancado.
- **Definition of Done:** DoD común.

---

## HU-007 — Entorno local de desarrollo

- **Objetivo:** que cualquier agente/desarrollador levante dependencias locales con un comando.
- **Descripción:** `docker-compose.dev.yml` con `postgres` y `mosquitto` (config de desarrollo con dynsec y listener sin TLS en `127.0.0.1`), puertos solo en `127.0.0.1`; `.local-secrets/` de ejemplo generado por un script; guía en README.
- **Criterios de aceptación:**
  1. `docker compose -f docker-compose.dev.yml up -d` + `dotnet run --project src/Jarvis.Api` → API ready.
  2. Ningún puerto publicado en `0.0.0.0`.
  3. Secretos de desarrollo fuera de git.
- **Dependencias:** HU-003 (la configuración de Mosquitto definitiva llega en HU-020; aquí basta un broker arrancable).
- **Consideraciones técnicas:** reutilizar los mismos archivos de config de Mosquitto que producción donde sea posible, cambiando solo listeners/TLS.
- **Pruebas esperadas:** verificación manual documentada; CI no depende de este compose (usa Testcontainers).
- **Definition of Done:** DoD común + sección "Desarrollo local" en README.
