# Jarvis Cloud Core

Implementación de la **Phase 1 — Cloud Core MVP** de Jarvis según [docs/](docs/README.md), que sigue siendo la fuente de verdad del diseño. El estado de cada historia de usuario está en [docs/09-backlog/implementation-status.md](docs/09-backlog/implementation-status.md).

El Cloud transporta **tareas estructuradas** entre devices (ESP32, Windows, mobile, web). No ejecuta comandos, no expone ejecución genérica y no conoce ningún LLM ([ADR-010](docs/08-decisions/ADR-010-llm-independence.md)).

## Estructura

```text
src/
  Jarvis.Domain/          Entidades y reglas: JarvisTask (máquina de estados), Device, User, grants, eventos
  Jarvis.Contracts/       Protocolo MQTT v1 (envelope, topics, mensajes) y DTOs REST — compartido con simuladores
  Jarvis.Application/     Casos de uso, autorización de negocio, procesador MQTT de entrada, presencia, sweepers
  Jarvis.Infrastructure/  EF Core + PostgreSQL, migraciones, MQTTnet, Mosquitto Dynamic Security, JWT, hashing
  Jarvis.Api/             Host: REST /api/v1, hub SignalR, health, métricas, background services, subcomandos
tools/
  Jarvis.Simulators/      jarvis-sim: Windows Simulator (ejecutor) y ESP32 Simulator (origen)
tests/
  Jarvis.Domain.Tests/        Unitarias del dominio (sin infraestructura)
  Jarvis.Application.Tests/   Arquitectura, contratos vs. documentación, modelo EF vs. migración, helpers
  Jarvis.IntegrationTests/    API + PostgreSQL + Mosquitto reales (Testcontainers)
  Jarvis.E2ETests/            Escenarios E2E-01 … E2E-10 con simuladores por MQTT/TLS
deploy/
  docker-compose.yml          Producción: jarvis-api (red del host, 127.0.0.1) y mosquitto; PostgreSQL y nginx nativos
  docker-compose.dev.yml      Desarrollo: postgres + mosquitto en 127.0.0.1
  mosquitto/ nginx/ postgres/ Configuración versionada (sin secretos)
  scripts/                    deploy.sh (despliegue completo), secretos, certbot, backups, monitoreo
Dockerfile                    Imagen de producción (chiseled, no-root)
```

## Requisitos

- .NET SDK 10.0 (ver `global.json`).
- Docker (Desktop o Engine) para el entorno local y para las pruebas de integración/E2E.

## Compilar y probar

```bash
dotnet build Jarvis.slnx
dotnet test --solution Jarvis.slnx                     # 158 pruebas; las de integración/E2E requieren Docker
dotnet test --project tests/Jarvis.Domain.Tests        # solo unitarias, sin Docker
```

Las pruebas de integración y E2E levantan PostgreSQL 17 y Mosquitto 2.0.22 con la **misma configuración de producción** de `deploy/` (incluido TLS en 8883 con una CA de prueba). Si Docker no está disponible, esas pruebas se omiten con un mensaje explicativo.

## Desarrollo local

```bash
# 1. Secretos locales (fuera de git). En Windows sin Git Bash: deploy/scripts/generate-dev-secrets.ps1
sh deploy/scripts/generate-secrets.sh .local-secrets

# 2. PostgreSQL (127.0.0.1:55432) y Mosquitto (127.0.0.1:1883)
docker compose -f deploy/docker-compose.dev.yml up -d

# 3. Una sola vez: Dynamic Security (roles, ACL deny por defecto, client jarvis-cloud) y esquema
cd src/Jarvis.Api
export ASPNETCORE_ENVIRONMENT=Development
dotnet run -- broker-bootstrap --host localhost --port 1883 \
  --admin-password-file ../../.local-secrets/mosquitto_dynsec_admin_password \
  --cloud-password-file ../../.local-secrets/mqtt_cloud_password --delete-admin
dotnet run -- migrate
echo "una-frase-larga-y-unica" | dotnet run -- bootstrap-owner --email owner@example.com --display-name Owner

# 4. API en http://localhost:8080 (métricas en http://localhost:8081/metrics, OpenAPI en /openapi/v1.json)
dotnet run
```

Los puertos se pueden cambiar con `JARVIS_DEV_PG_PORT` y `JARVIS_DEV_MQTT_PORT` (ajustar también `appsettings.Development.json`).

## Demo del MVP con los simuladores

Con la API corriendo:

```bash
TOKEN=$(curl -s -X POST localhost:8080/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email":"owner@example.com","password":"una-frase-larga-y-unica"}' | jq -r .accessToken)

# Registrar devices (el secreto se muestra una sola vez) y conceder OPEN_CHROME del ESP32 al Windows
curl -s -X POST localhost:8080/api/v1/devices -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"windows-sim","type":"Windows"}' > win.json
curl -s -X POST localhost:8080/api/v1/devices -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"esp32-sim","type":"ESP32"}' > esp.json
WIN=$(jq -r .device.id win.json); ESP=$(jq -r .device.id esp.json)
jq -r .credential.secret win.json > win.secret; jq -r .credential.secret esp.json > esp.secret
curl -s -X POST localhost:8080/api/v1/devices/$ESP/grants -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d "{\"targetDeviceId\":\"$WIN\",\"taskType\":\"OPEN_CHROME\"}"

# Terminal 1: ejecutor
dotnet run --project tools/Jarvis.Simulators -- windows --device-id $WIN --secret-file win.secret --port 1883 --no-tls
# Terminal 2: botón (--press-once o interactivo con la tecla b)
dotnet run --project tools/Jarvis.Simulators -- esp32 --device-id $ESP --secret-file esp.secret --target $WIN --port 1883 --no-tls --press-once
```

Salida esperada del ESP32: `Enviado` → `Dispatched` → `Delivered` → `Running` → `Completed - Chrome opened (simulated)`.
Historia completa: `GET /api/v1/events?correlationId=<id>`.

Contra producción los simuladores usan TLS en `${JARVIS_DOMAIN}:8883` (omitir `--no-tls`; `--ca-file` solo para CAs propias). Flags de prueba del Windows Simulator: `--drop-acks N`, `--disconnect-before-ack`, `--crash-after-start`, `--pause-heartbeats S`, `--busy`, `--fail-with CODE`, `--slow S`, `--reject-type TYPE`.

## Subcomandos de `jarvis-api`

| Comando | Uso |
|---|---|
| *(sin argumentos)* | Servidor HTTP (8080) + métricas (8081) + cliente MQTT + background services |
| `migrate` | Aplica migraciones con `ConnectionStrings:PostgresMigrator` (rol `jarvis_migrator`) |
| `bootstrap-owner --email <e> [--display-name <n>]` | Crea un owner; la contraseña se lee de stdin |
| `broker-bootstrap --admin-password-file <f> --cloud-password-file <f> [--host] [--port] [--delete-admin]` | Inicializa Dynamic Security |
| `healthcheck [--url]` | Healthcheck del contenedor (la imagen no tiene shell ni curl) |

## Producción

PostgreSQL (≥ 15) y nginx son **nativos** en el servidor; Docker solo ejecuta `jarvis-api` y `mosquitto` ([ADR-011](docs/08-decisions/ADR-011-native-postgres-nginx.md)). Un único script idempotente hace todo el despliegue (secretos, aprovisionamiento de PostgreSQL, certificado, Mosquitto + Dynamic Security, migraciones, API, sitio nginx, renovación y owner inicial):

```bash
sudo cp deploy/.env.example deploy/.env && sudo editor deploy/.env   # JARVIS_DOMAIN, CERTBOT_EMAIL, PG_PORT
sudo ./deploy/scripts/deploy.sh --owner-email owner@example.com       # primer despliegue
sudo ./deploy/scripts/deploy.sh                                       # actualizaciones
```

Detalle de cada paso, backups, restauración y monitoreo: [docs/06-infrastructure/runbooks.md](docs/06-infrastructure/runbooks.md).
