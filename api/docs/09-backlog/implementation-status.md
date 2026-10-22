# Estado de implementación — Phase 1

Estado al 2026-09-19. Leyenda: ✅ implementada y verificada con pruebas automáticas · 🟡 implementada, verificación pendiente en el VPS · ⏸ post-MVP (no implementada, por diseño).

Pruebas: `dotnet test --solution Jarvis.slnx` → **158 pruebas en verde** (59 dominio, 61 aplicación/arquitectura/contratos, 28 integración con PostgreSQL + Mosquitto reales, 10 E2E). La demo del MVP (ESP32 Simulator → Cloud → Windows Simulator → Completed → ESP32) también se ejecutó con los binarios reales contra `docker-compose.dev.yml`.

| HU | Estado | Implementación | Verificación |
|---|---|---|---|
| HU-001 | ✅ | `Jarvis.slnx`, `Directory.Build.props` (warnings as errors, analizadores), `Directory.Packages.props` | `ArchitectureTests` |
| HU-002 | ✅ | `Application/Configuration/Options.cs` (DataAnnotations + `ValidateOnStart`), `SecretFileConfiguration` (`*File` → valor), `Password_File` en cadenas de conexión | Arranque de la API y subcomandos |
| HU-003 | ✅ | `JarvisDbContext`, `Configurations.cs` (snake_case, CHECKs, índices parciales), migración `InitialSchema` con GRANTs por rol, subcomando `migrate`, `deploy/postgres/provision.sql` | `PersistenceModelTests` (modelo = migración), integración |
| HU-004 | ✅ | `CorrelationMiddleware`, scopes de logging (API y MQTT), formatter JSON | `HostTests` |
| HU-005 | ✅ | `Problems` (RFC 9457, `urn:jarvis:problem:*`, `code`, `correlationId`, `traceId`), `UnhandledExceptionHandler`, 401/403/404/429 en Problem Details | `HostTests`, `ApplicationHelpersTests` |
| HU-006 | ✅ | `/health/live`, `/health/ready` (PostgreSQL+migraciones Unhealthy; MQTT, reconciliación y background services Degraded), subcomando `healthcheck` | `HostTests` |
| HU-007 | ✅ | `deploy/docker-compose.dev.yml`, `generate-secrets.sh` / `generate-dev-secrets.ps1`, `appsettings.Development.json` | Ejecutado manualmente |
| HU-008 | ✅ | `BootstrapOwnerHandler`, `PasswordPolicy`, subcomando `bootstrap-owner` (contraseña por stdin), evento `USER_CREATED` | Integración, ejecución real |
| HU-009 | ✅ | `LoginHandler`, `JwtTokenIssuer` (HS256, `current`/`previous`), lockout 5 fallos / 15 min | `AuthTests` |
| HU-010 | ✅ | `RefreshHandler` (rotación, detección de reutilización → revoca familia), `LogoutHandler` | `AuthTests` |
| HU-011 | ✅ | `Device`, `DeviceCredential`, `DeviceStatus` calculado | `DeviceTests` (dominio) |
| HU-012 | ✅ | `RegisterDeviceHandler` (secreto `jdv_`, HMAC con pepper, dynsec dentro de la transacción, 503 si el broker falla) | `DeviceTests` (integración) |
| HU-013 | ✅ | `DeviceQueryHandler` (filtros por estado calculado, cursor keyset) | `DeviceTests`, `TaskApiTests.Pagination_with_cursor` |
| HU-014 | ✅ | `DeviceAdminHandler.Rename/Disable/Enable` (desconexión inmediata vía dynsec) | `DeviceTests`, E2E-08 |
| HU-015 | ✅ | `DeviceAdminHandler.RevokeAsync` (credenciales, tareas → `Cancelled DEVICE_REVOKED`, grants, dynsec) | `DeviceTests`, E2E-07 |
| HU-016 | ✅ | `RotateCredentialAsync` (setClientPassword + disable/enable para cerrar la sesión) | `DeviceTests.Disable_disconnects_and_rotation_invalidates_old_secret` |
| HU-017 | ✅ | `DeviceTokenHandler`, `PrincipalStatusService` (caché 30 s) | `AuthTests.Device_token_lifecycle` |
| HU-018 | ✅ | `MosquittoDynsecAdmin` (correlationData, timeout, errores tipados) | Integración (registro, rotación, revocación) |
| HU-019 | ✅ | `BrokerReconciler` + `BrokerReconciliationHostedService` (huérfanos en 2 ciclos, disable/enable, `brokerSynced`) | `MaintenanceTests` |
| HU-020 | ✅ | `deploy/mosquitto/*` (TLS 8883, interno 1883, retain off, límites), `DynsecBootstrapper` + `broker-bootstrap`, **rol por device con topics literales** (ver desviaciones) | `BrokerAclTests`, E2E-05 |
| HU-021 | ✅ | `MqttCloudConnection` (sesión persistente, PUBACK manual tras commit, particiones por device, reconexión) | E2E-10 (mensaje procesado tras reiniciar la API) |
| HU-022 | ✅ | `MqttInboundProcessor` (7 reglas de validación, `SECURITY_VIOLATION`, `STALE_MESSAGE`, rate limit) | `BrokerAclTests.Envelope_device_mismatch…`, `ProtocolDocumentationTests` |
| HU-023 | ✅ | `MqttDeviceMessagePublisher` | Integración/E2E |
| HU-024 | ✅ | `JarvisTask` (T1-T17, avance monótono, relleno de `TASK_DELIVERED`) | `TaskStateMachineTests` (incluye prueba de propiedades) |
| HU-025 | ✅ | `GrantHandler` | `DeviceTests.Grants_validation` |
| HU-026 | ✅ | `CreateTaskHandler` (orden de autorización documentado, idempotencia con `request_hash`, `TASK_REJECTED`) | `TaskApiTests` (incluye concurrencia con la misma key) |
| HU-027 | ✅ | `DeviceRequestHandler` (`task.create`, `request.accepted/rejected`, `duplicate`) | E2E-01, E2E-05 |
| HU-028 | ✅ | `TaskDispatchService` (`FOR UPDATE SKIP LOCKED`) + `TaskDispatcherService` (señal + 5 s) | E2E-01/02 |
| HU-029 | ✅ | `DeviceTaskUpdateHandler` (reintento ante conflicto `xmin`, `task.update.ack`, `late_terminal_update`) | E2E-01/03/04 |
| HU-030 | ✅ | `TaskSweepService.SweepAckTimeoutsAsync` | E2E-03 (3 intentos, 1 ejecución) |
| HU-031 | ✅ | `TaskSweepService.SweepExpirationsAsync` | E2E-06; T16 en pruebas de dominio |
| HU-032 | ✅ | `CancelTaskHandler` (REST y `task.cancel_request`), reenvío de `task.cancel` cada 60 s | E2E-09, `TaskApiTests` |
| HU-033 | ✅ | `TaskQueryHandler` | `TaskApiTests` |
| HU-034 | ✅ | `SourceDeviceTaskNotifier`, `task.get` | E2E-01/06/09 |
| HU-035 | ✅ | `PresenceHandler.HandleHelloAsync` (reconciliación con `openTaskIds`) | E2E-04, dominio |
| HU-036 | ✅ | Heartbeat, goodbye, LWT con `sessionId` | E2E-04/08 |
| HU-037 | ✅ | `PresenceSweepService` + gauges | E2E-08 |
| HU-038 | ✅ | `JarvisDbContext.SaveChangesAsync` (eventos en la misma transacción, advisory lock por owner, notificación post-commit) | Integración/E2E (historial por `correlationId`) |
| HU-039 | ✅ | `EventQueryHandler` | `DeviceTests`, E2E-01 |
| HU-040 | ✅ | `RetentionService` + `RetentionStore` (rol `jarvis_maintenance`) | `MaintenanceTests` |
| HU-041 | ✅ | `EventsHub` (solo users, group por owner) | `RealtimeTests` |
| HU-042 | ✅ | `RealtimeEventSubscriber` + `SignalRRealtimeNotifier` | `RealtimeTests` (orden, aislamiento, catch-up) |
| HU-043 | ✅ | Políticas `UserOnly`/`DeviceOnly`/`UserOrDevice`, filtro global por owner, 404 para recursos ajenos | `CrossOwnerAuthorizationTests` |
| HU-044 | ✅ | Rate limiting REST (auth, creación de tareas) y MQTT (mensajes/s, `task.create`/min, `message_flood`) | `ApplicationHelpersTests` |
| HU-045 | ✅ | Forwarded headers por CIDR, CORS por lista, sin Swagger en producción, límite 64 KiB, `/metrics` solo en 8081 | `HostTests` |
| HU-046 | ✅ | Política de logging sin secretos ni payloads | `RedactionTests` |
| HU-047 | ✅ | `Jarvis.Contracts` | `ProtocolDocumentationTests` (todos los ejemplos JSON de `mqtt-protocol.md`) |
| HU-048 | ✅ | `WindowsSimulator` (inbox/outbox SQLite, W1-W10, flags de prueba) | E2E, `SimulatorStoreTests` |
| HU-049 | ✅ | `Esp32Simulator` | E2E |
| HU-050 | ✅ | `Jarvis.E2ETests` (E2E-01 … E2E-10) | 10/10 en verde |
| HU-051 | 🟡 | `Dockerfile` (multi-stage, chiseled, no-root, `healthcheck`) | Imagen construida y probada con `--read-only --cap-drop ALL`. Tamaño 195 MB (meta orientativa 150 MB). Escaneo `trivy` configurado en CI, no ejecutado localmente |
| HU-052 | 🟡 | nginx **nativo** (ADR-011): `deploy/nginx/jarvis-site.conf.template` instalado por `deploy.sh`, certbot webroot + renewal hook | `nginx -t` válido dentro del `nginx.conf` estándar. SSL Labs, WSS real y `certbot renew --dry-run` requieren el dominio |
| HU-053 | 🟡 | `deploy/docker-compose.yml` (solo `jarvis-api` en red del host sobre 127.0.0.1 y `mosquitto`) + `deploy/scripts/deploy.sh` (despliegue idempotente de 10 pasos) + `deploy/postgres/provision.sql` (PostgreSQL nativo ≥ 15) | `docker compose config` válido (8883 público, 1883 solo loopback); `shellcheck` sin avisos; `provision.sql` probado dos veces contra PostgreSQL 15 y 16 con migración como `jarvis_migrator` y verificación de privilegios. Ejecución completa del script pendiente en el servidor |
| HU-054 | 🟡 | [Runbook del VPS](../06-infrastructure/runbooks.md#1-requisitos-del-servidor) | Checklist a ejecutar en el VPS |
| HU-055 | 🟡 | `deploy/scripts/backup.sh`, [runbook de restauración](../06-infrastructure/runbooks.md#5-restauración) | Requiere proveedor de almacenamiento (D5) y simulacro de restauración |
| HU-056 | 🟡 | `deploy/scripts/monitor.sh` | Requiere canal de alertas (D6) y ejecución en el VPS |
| HU-057 | ⏸ | — | Post-MVP (Phase 3) |

## Precisiones y desviaciones respecto al diseño

- **Despliegue (ADR-011):** en producción PostgreSQL y nginx son nativos del servidor; ver [ADR-011](../08-decisions/ADR-011-native-postgres-nginx.md).

Ninguna cambia contratos ni ADRs; todas están reflejadas en el documento correspondiente.

| Tema | Decisión de implementación | Documento actualizado |
|---|---|---|
| ACL de Mosquitto | Se usa el **fallback previsto en ADR-003**: un rol dynsec por device (`jarvis-device-{deviceId}`) con topics literales, en lugar de un rol compartido con `%u`. No depende de la sustitución de patrones y la prueba de aislamiento pasa. El rol se borra junto con el client. | [ADR-003](../08-decisions/ADR-003-mqtt-device-communication.md), [MQTT §8](../03-protocols/mqtt-protocol.md#8-autenticación-autorización-y-acl) |
| ACL por defecto | `publishClientReceive` también es deny por defecto; los roles conceden recepción explícita (`down/#` al device, `up/#` y `$CONTROL` al cloud). | [MQTT §8](../03-protocols/mqtt-protocol.md#8-autenticación-autorización-y-acl) |
| Reintentos de EF Core | Sin `EnableRetryOnFailure`: el DbContext abre transacciones propias (eventos + advisory lock), incompatibles con la estrategia de reintento. Los errores transitorios se cubren con los reintentos de los background services y del procesador MQTT. | [Persistence Strategy](../04-data/persistence-strategy.md) |
| Índice de idempotencia | `UNIQUE (owner_id, requested_by_type, requested_by_user_id, source_device_id, idempotency_key) NULLS NOT DISTINCT WHERE idempotency_key IS NOT NULL` — equivalente al `COALESCE` del diseño y expresable en EF Core. | [Database Design](../04-data/database-design.md) |
| Cierre de conexiones SignalR | Se cierran al expirar el token (`CloseOnAuthenticationExpiration`), más estricto que "exp + 1 h". | [Realtime Protocol](../03-protocols/realtime-protocol.md) |
| Límite de `/auth/*` | Configurable (`Jarvis:Security:AuthRequestsPerMinute`, 20/min por IP por defecto) además del límite de nginx. | [Security Architecture](../05-security/security-architecture.md#rate-limiting) |
| Puerto PostgreSQL de desarrollo | 55432 en `127.0.0.1` para no chocar con instalaciones locales de PostgreSQL. | [Docker Strategy](../06-infrastructure/docker-strategy.md) |
| Healthcheck de Mosquitto | `nc -z` sobre 1883 y 8883 (sin credenciales); la salud funcional la reporta la API. | [Docker Strategy](../06-infrastructure/docker-strategy.md) |
| Lista de contraseñas comunes | Lista local reducida (~100 entradas ≥ 12 caracteres), ampliable en `Application/Auth/common-passwords.txt`. El diseño pedía top 10 000. | [Security Architecture](../05-security/security-architecture.md#autenticación-de-users) |

## Phase 2 — Memoria, sincronización y jobs (2026-10-22)

Leyenda igual que arriba. Detalle y escenarios de extremo a extremo en [`../../../docs/PROGRESS.md`](../../../docs/PROGRESS.md).

| Módulo | Estado | Implementación | Verificación |
|---|---|---|---|
| Pruebas sin Docker | ✅ | `TestServices.cs`: PostgreSQL (pgvector) y Mosquitto nativos con `JARVIS_TEST_SERVICES=native`; contenedores siguen siendo el valor por defecto | Suite completa en verde con servicios nativos |
| Esquema `memory` | ✅ | Migración `MemorySyncAndJobs` (aditiva): recuerdos, versiones, relaciones, embeddings (`real[]`), experiencias, habilidades, propuestas, sync, conflictos, jobs; GRANT condicionales por rol | `PersistenceModelTests`, `check-provision.sh` |
| Sync `jarvis.sync/1` | ✅ | `/api/v1/sync/{push,pull,device,conflicts}`; recibos idempotentes, fusión a tres bandas, tombstones con purga, límite 1 MiB solo en `push` | `SyncTests`, `SyncContractTests` (vectores compartidos con Rust) |
| Memoria y grafo | ✅ | `/api/v1/memories` (listar, versiones, búsqueda vectorial con pgvector o en la aplicación, exportación con hash) | `SyncTests` |
| Experiencias y habilidades | ✅ | `/api/v1/experiences`, `/api/v1/skills` (propuestas aceptadas por el owner; nunca se ejecutan en el servidor) | `SyncTests` |
| Jobs + `jarvis-worker` | ✅ | `/api/v1/jobs`, catálogo cerrado (`report.summary`, `memory.consolidate`, `sync.compact`, `memory.purge_expired` con aprobación, `backup.verify`, `http.check` con allowlist) | `JobTests`, `WorkerProcessTests` |
| Despliegue | 🟡 | `Dockerfile.worker`, servicio `jarvis-worker`, rol `jarvis_worker`, `inspect-vps.sh`, nginx `sync/push` 1 MiB, `SHA256SUMS` en backups | Pruebas locales; **no ejecutado en el VPS** ([contabo-deployment.md](../06-infrastructure/contabo-deployment.md)) |
