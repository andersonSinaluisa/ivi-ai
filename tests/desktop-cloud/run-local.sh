#!/usr/bin/env bash
# Prueba de integración Escritorio ↔ Cloud Core con servicios REALES y locales (Linux):
# PostgreSQL (con pgvector) + Mosquitto (Dynamic Security) + jarvis-api, todo en 127.0.0.1 y en una carpeta temporal.
# Después ejecuta las pruebas de Rust de assistant/…/tests/desktop_cloud.rs (escenarios A, B, C, D, F, G y H).
#
# Requisitos: binarios de PostgreSQL >= 15 con la extensión vector, mosquitto + mosquitto_ctrl, .NET SDK 10, cargo, openssl.
# No toca ninguna instalación existente: puertos y datos propios; todo se borra al terminar.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d /tmp/jarvis-desktop-cloud.XXXXXX)"
chmod 755 "$WORK"
PG_BIN="${PG_BIN:-$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1 || true)}"
[ -x "$PG_BIN/initdb" ] || { echo "No se encontró initdb (define PG_BIN)" >&2; exit 1; }
PG_PORT="${PG_PORT:-55499}"
MQ_PORT="${MQ_PORT:-18883}"
API_PORT="${API_PORT:-18080}"
EMAIL="it-owner@example.com"
PASSWORD="Integracion-Escritorio-Cloud-$(openssl rand -hex 4)"
API_PID=""
MQ_PID=""
WORKER_PID=""

as_postgres() { if [ "$(id -u)" = "0" ]; then runuser -u postgres -- "$@"; else "$@"; fi; }

cleanup() {
  [ -n "$API_PID" ] && kill "$API_PID" 2>/dev/null || true
  [ -n "$WORKER_PID" ] && kill "$WORKER_PID" 2>/dev/null || true
  [ -n "$MQ_PID" ] && kill "$MQ_PID" 2>/dev/null || true
  as_postgres "$PG_BIN/pg_ctl" -D "$WORK/pg" -m immediate -w stop >/dev/null 2>&1 || true
  if [ "${KEEP_WORK:-0}" != "1" ]; then rm -rf "$WORK"; else echo "Datos conservados en $WORK"; fi
}
trap cleanup EXIT

echo "==> PostgreSQL en 127.0.0.1:$PG_PORT"
mkdir -p "$WORK/pg"
[ "$(id -u)" = "0" ] && chown postgres:postgres "$WORK/pg"
as_postgres "$PG_BIN/initdb" -D "$WORK/pg" -U postgres -A trust -E UTF8 --no-instructions >/dev/null
as_postgres "$PG_BIN/pg_ctl" -D "$WORK/pg" -l "$WORK/pg/server.log" -w \
  -o "-p $PG_PORT -k $WORK/pg -c listen_addresses=127.0.0.1" start >/dev/null
psql -h 127.0.0.1 -p "$PG_PORT" -U postgres -v ON_ERROR_STOP=1 -q -c "CREATE DATABASE jarvis"
# Como provision.sql en producción: las extensiones las crea el superusuario.
psql -h 127.0.0.1 -p "$PG_PORT" -U postgres -d jarvis -v ON_ERROR_STOP=1 -q \
  -c "CREATE EXTENSION IF NOT EXISTS citext" -c "CREATE EXTENSION IF NOT EXISTS vector"

echo "==> Mosquitto (Dynamic Security) en 127.0.0.1:$MQ_PORT"
mkdir -p "$WORK/mq" "$WORK/secrets"
openssl rand -hex 16 > "$WORK/secrets/mq_admin"
openssl rand -hex 16 > "$WORK/secrets/mq_cloud"
openssl rand -base64 48 | tr -d '\n' > "$WORK/secrets/jwt"
openssl rand -base64 48 | tr -d '\n' > "$WORK/secrets/pepper"
PLUGIN=""
for candidate in /usr/lib/*/mosquitto_dynamic_security.so /usr/lib/mosquitto_dynamic_security.so /usr/local/lib/mosquitto_dynamic_security.so; do
  [ -f "$candidate" ] && PLUGIN="$candidate" && break
done
[ -n "$PLUGIN" ] || { echo "No se encontró mosquitto_dynamic_security.so" >&2; exit 1; }
mosquitto_ctrl dynsec init "$WORK/mq/dynamic-security.json" bootstrap-admin "$(cat "$WORK/secrets/mq_admin")" >/dev/null
cat > "$WORK/mq/mosquitto.conf" <<EOF
per_listener_settings false
allow_anonymous false
plugin $PLUGIN
plugin_opt_config_file $WORK/mq/dynamic-security.json
listener $MQ_PORT 127.0.0.1
persistence false
$( [ "$(id -u)" = "0" ] && echo "user root" )
EOF
mosquitto -c "$WORK/mq/mosquitto.conf" > "$WORK/mq/log" 2>&1 &
MQ_PID=$!
sleep 1

echo "==> Cloud Core (jarvis-api) en http://127.0.0.1:$API_PORT"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
dotnet build "$ROOT/api/src/Jarvis.Api/Jarvis.Api.csproj" -v q -nologo >/dev/null
API_DLL="$ROOT/api/src/Jarvis.Api/bin/Debug/net10.0/jarvis-api.dll"
export ASPNETCORE_ENVIRONMENT=Production
export Logging__Console__FormatterName=simple
export ConnectionStrings__Postgres="Host=127.0.0.1;Port=$PG_PORT;Database=jarvis;Username=postgres;Password=unused"
export ConnectionStrings__PostgresMigrator="$ConnectionStrings__Postgres"
export Jwt__SigningKeyFile="$WORK/secrets/jwt"
export Jarvis__DeviceSecretPepperFile="$WORK/secrets/pepper"
export Jarvis__PublicBaseUrl="http://127.0.0.1:$API_PORT"
export Jarvis__Mqtt__Host=127.0.0.1 Jarvis__Mqtt__Port="$MQ_PORT" Jarvis__Mqtt__PasswordFile="$WORK/secrets/mq_cloud"
export Jarvis__Mqtt__PublicHost=127.0.0.1 Jarvis__Mqtt__PublicPort="$MQ_PORT"
export Jarvis__Security__AuthRequestsPerMinute=1000
dotnet "$API_DLL" broker-bootstrap --host 127.0.0.1 --port "$MQ_PORT" \
  --admin-password-file "$WORK/secrets/mq_admin" --cloud-password-file "$WORK/secrets/mq_cloud" --delete-admin >/dev/null
dotnet "$API_DLL" migrate >/dev/null
echo "$PASSWORD" | dotnet "$API_DLL" bootstrap-owner --email "$EMAIL" --display-name "Integración" >/dev/null
start_api() {
  ASPNETCORE_URLS="http://127.0.0.1:$API_PORT" dotnet "$API_DLL" >> "$WORK/api.log" 2>&1 &
  API_PID=$!
  for _ in $(seq 1 60); do
    curl -sf "http://127.0.0.1:$API_PORT/api/v1/health" >/dev/null && return 0
    sleep 0.5
  done
  echo "La API no arrancó" >&2; tail -20 "$WORK/api.log" >&2; return 1
}
start_api
curl -sf "http://127.0.0.1:$API_PORT/api/v1/health"; echo

echo "==> Pruebas del escritorio contra el Cloud Core (escenarios A, B, C, D, F, G, H)"
(cd "$ROOT/assistant/apps/desktop/src-tauri" && \
  JARVIS_CLOUD_IT_URL="http://127.0.0.1:$API_PORT" JARVIS_CLOUD_IT_EMAIL="$EMAIL" JARVIS_CLOUD_IT_PASSWORD="$PASSWORD" \
  cargo test --test desktop_cloud -- --test-threads=1 "$@")

echo "==> Escenario E: reiniciar el Cloud Core sin perder tareas persistentes"
json() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }
TOKEN=$(curl -sf -X POST "http://127.0.0.1:$API_PORT/api/v1/auth/login" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | json "['accessToken']")
JOB=$(curl -sf -X POST "http://127.0.0.1:$API_PORT/api/v1/jobs" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"type":"report.summary","name":"Resumen tras reinicio"}' | json "['id']")
echo "   job $JOB creado con el worker apagado; se reinicia la API"
kill "$API_PID"; wait "$API_PID" 2>/dev/null || true
start_api
curl -sf "http://127.0.0.1:$API_PORT/api/v1/jobs/$JOB" -H "Authorization: Bearer $TOKEN" >/dev/null
dotnet build "$ROOT/api/src/Jarvis.Worker/Jarvis.Worker.csproj" -v q -nologo >/dev/null
Jarvis__Worker__PollSeconds=1 Jarvis__Worker__HeartbeatPath="$WORK/worker.alive" Jarvis__Worker__BackupDirectory="$WORK" \
  dotnet "$ROOT/api/src/Jarvis.Worker/bin/Debug/net10.0/jarvis-worker.dll" > "$WORK/worker.log" 2>&1 &
WORKER_PID=$!
STATUS=""
for _ in $(seq 1 40); do
  STATUS=$(curl -sf "http://127.0.0.1:$API_PORT/api/v1/jobs/$JOB/runs" -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import json,sys; i=json.load(sys.stdin)['items']; print(i[0]['status'] if i else '')")
  [ "$STATUS" = "succeeded" ] && break
  sleep 0.5
done
if [ "$STATUS" != "succeeded" ]; then echo "   FALLO: la ejecución quedó en '$STATUS'" >&2; tail -20 "$WORK/worker.log" >&2; exit 1; fi
dotnet "$ROOT/api/src/Jarvis.Worker/bin/Debug/net10.0/jarvis-worker.dll" healthcheck --Jarvis:Worker:HeartbeatPath="$WORK/worker.alive"
echo "   ok: el job sobrevivió al reinicio de la API y el worker lo ejecutó (healthcheck del worker = 0)"
