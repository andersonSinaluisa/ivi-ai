# Docker Strategy

> **Producción (ADR-011):** PostgreSQL y nginx son **nativos** en el host; Docker solo ejecuta `jarvis-api` (red del host, escucha en 127.0.0.1) y `mosquitto`. Ver [ADR-011](../08-decisions/ADR-011-native-postgres-nginx.md) y [Runbooks](runbooks.md). Las secciones que describen nginx/PostgreSQL en contenedores quedan como referencia del diseño original y siguen aplicando a desarrollo/pruebas.

Diseño del futuro `docker-compose.yml` de producción. **No** se crea el archivo en esta fase; se implementa en [HU-053](../09-backlog/stories/EPIC-11-deployment.md).

## Servicios

| Servicio | Imagen | Redes | Puertos publicados | Volúmenes | Secrets |
|---|---|---|---|---|---|
| `nginx` | `nginx:1.27-alpine` (tag fijado por digest) | `edge` | `80:80`, `443:443` | `./config/nginx:/etc/nginx:ro`, `/etc/letsencrypt:/etc/letsencrypt:ro`, `/var/www/certbot:/var/www/certbot:ro`, `nginx-logs:/var/log/nginx` | — |
| `jarvis-api` | `jarvis-api:{version}` (build propio) | `edge`, `backend` | ninguno | ninguno (stateless) | `db_app_password`, `db_maintenance_password`, `jwt_signing_key`, `device_secret_pepper`, `mqtt_cloud_password` |
| `postgres` | `pgvector/pgvector:0.8.1-pg17` (PostgreSQL 17 + pgvector) | `backend` | **ninguno** | `pg-data:/var/lib/postgresql/data`, `./config/postgres/init:/docker-entrypoint-initdb.d:ro` | `postgres_superuser_password`, `db_app_password`, `db_migrator_password`, `db_maintenance_password` (para el script de init de roles) |
| `mosquitto` | `eclipse-mosquitto:2.0.22` (entrypoint `deploy/mosquitto/docker-entrypoint.sh` que inicializa dynsec con un admin temporal) | `backend`, `mqtt-public` | `8883:8883` | `./mosquitto/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro`, `mosquitto-data:/mosquitto/data`, `mosquitto-log:/mosquitto/log`, `./config/mosquitto/certs:/mosquitto/certs:ro` (copia de los certificados hecha por el deploy-hook de certbot, ver [Nginx](nginx.md#lets-encrypt)) | — |

Redis: **no existe** en el MVP ([ADR-008](../08-decisions/ADR-008-redis-decision.md)).

## Imagen de `jarvis-api`

- Multi-stage: `mcr.microsoft.com/dotnet/sdk:10.0` para build → `mcr.microsoft.com/dotnet/aspnet:10.0-noble-chiseled` (sin shell, sin package manager, usuario no-root `app` UID 1654) para runtime.
- `ASPNETCORE_URLS=http://+:8080` (API y hub). Métricas en `http://+:8081/metrics`, puerto no publicado ni enrutado por nginx ([Observability](observability.md#métricas)).
- Healthcheck: la imagen chiseled no tiene `curl`; el binario acepta `jarvis-api healthcheck` que hace GET a `http://localhost:8080/health/ready` y sale con 0/1.
- Subcomandos del mismo binario: (sin args) servidor · `migrate` · `bootstrap-owner` · `healthcheck`.
- Etiquetado: `jarvis-api:{semver}` + `:{git-sha}`. Nunca `latest` en producción.

## Variables de entorno (sin secretos)

| Variable | Servicio | Ejemplo |
|---|---|---|
| `ASPNETCORE_ENVIRONMENT` | api | `Production` |
| `Jarvis__PublicBaseUrl` | api | `https://${JARVIS_DOMAIN}` |
| `Jarvis__Mqtt__PublicHost` / `PublicPort` | api | `${JARVIS_DOMAIN}` / `8883` (se devuelven al registrar devices) |
| `ConnectionStrings__Postgres` | api | `Host=postgres;Database=jarvis;Username=jarvis_app;Password_File=/run/secrets/db_app_password` (la app resuelve `*_File`) |
| `Jarvis__Mqtt__Host` / `Port` / `Username` / `PasswordFile` | api | `mosquitto` / `1883` / `jarvis-cloud` / `/run/secrets/mqtt_cloud_password` |
| `Jwt__SigningKeyFile` | api | `/run/secrets/jwt_signing_key` |
| `Jarvis__DeviceSecretPepperFile` | api | `/run/secrets/device_secret_pepper` |
| `Jarvis__Tasks__AckTimeoutSeconds` etc. | api | Defaults de [Task State Machine](../02-domain/task-state-machine.md#tiempos-por-defecto) |
| `Logging__Console__FormatterName` | api | `json` |
| `POSTGRES_PASSWORD_FILE` | postgres | `/run/secrets/postgres_superuser_password` |
| `JARVIS_DOMAIN` | compose (`.env` sin secretos) | `jarvis.example.com` |

## Health checks

| Servicio | Comando | Intervalo / timeout / retries / start_period |
|---|---|---|
| postgres | `pg_isready -U postgres -d jarvis` | 10 s / 5 s / 5 / 30 s |
| mosquitto | `nc -z localhost 1883 && nc -z localhost 8883` (busybox; sin credenciales, solo verifica listeners). La salud funcional del broker la verifica la API (check `Degraded` de su conexión MQTT). | 30 s / 5 s / 3 / 20 s |
| jarvis-api | `jarvis-api healthcheck` | 15 s / 5 s / 3 / 30 s |
| nginx | `wget -qO- http://localhost/nginx-health` (location interna que responde 200) | 30 s / 5 s / 3 / 10 s |

`depends_on` con `condition: service_healthy`: api → postgres, mosquitto; nginx → api (`service_started`, para que nginx pueda servir el desafío ACME aunque la API esté caída).

## Restart policies

Todos `restart: unless-stopped`. El paso `migrate` es `docker compose run --rm` (no servicio permanente).

## Límites de recursos

| Servicio | `mem_limit` | `cpus` | `pids_limit` |
|---|---|---|---|
| postgres | 2g | 2.0 | 200 |
| jarvis-api | 1g | 2.0 | 200 |
| mosquitto | 256m | 1.0 | 100 |
| nginx | 128m | 0.5 | 100 |

## Endurecimiento de contenedores

| Opción | nginx | api | postgres | mosquitto |
|---|---|---|---|---|
| Usuario no-root | worker no-root (master root para bind 80/443) | ✓ | ✓ (imagen) | ✓ (imagen, `mosquitto`) |
| `read_only: true` + `tmpfs` | ✓ (`/var/cache/nginx`, `/var/run`) | ✓ (`/tmp`) | ✗ | ✗ |
| `security_opt: no-new-privileges:true` | ✓ | ✓ | ✓ | ✓ |
| `cap_drop: [ALL]` | ✓ + `NET_BIND_SERVICE`, `CHOWN`, `SETUID`, `SETGID` | ✓ | ✗ (necesita varias) | ✓ + `NET_BIND_SERVICE` si aplica |

## Logging

Driver `json-file` con `max-size: 10m`, `max-file: 5` en todos los servicios (máx. ~200 MB). Centralización de logs: post-MVP.

## Volúmenes

| Volumen | Contenido | Backup |
|---|---|---|
| `pg-data` | Datos PostgreSQL | Vía `pg_dump` (no copia de archivos) |
| `mosquitto-data` | `dynamic-security.json`, persistencia de sesiones | Copia diaria del archivo dynsec |
| `mosquitto-log` | Logs del broker | No |
| `nginx-logs` | Access/error logs | No |

## Compose de desarrollo

`deploy/docker-compose.dev.yml` con solo `postgres` (`127.0.0.1:55432`, para no chocar con un PostgreSQL local) y `mosquitto` (`127.0.0.1:1883`, sin TLS), sin nginx. La API corre con `dotnet run`. Implementación real: `deploy/docker-compose.yml`, `Dockerfile` (imagen de 195 MB: runtime chiseled ~110 MB + EF Core, Npgsql, OpenTelemetry) y los scripts de `deploy/scripts/`. Procedimientos en [Runbooks](runbooks.md).
