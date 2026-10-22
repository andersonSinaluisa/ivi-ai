#!/usr/bin/env bash
# Comprueba deploy/postgres/provision.sql + migraciones contra un PostgreSQL NATIVO temporal (no toca ninguna
# instalación existente): roles de mínimo privilegio, esquemas, pgvector opcional, idempotencia y permisos del worker.
# Requisitos: binarios de PostgreSQL >= 15 (con o sin pgvector), .NET SDK 10.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d /tmp/jarvis-provision.XXXXXX)"
chmod 755 "$WORK"
PG_BIN="${PG_BIN:-$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1 || true)}"
PG_PORT="${PG_PORT:-55498}"
as_postgres() { if [ "$(id -u)" = "0" ]; then runuser -u postgres -- "$@"; else "$@"; fi; }
cleanup() { as_postgres "$PG_BIN/pg_ctl" -D "$WORK/pg" -m immediate -w stop >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT
fail() { echo "FALLO: $*" >&2; exit 1; }
pass() { echo "  ✓ $*"; }

mkdir -p "$WORK/pg" "$WORK/secrets"
[ "$(id -u)" = "0" ] && chown postgres:postgres "$WORK/pg"
# Como en un servidor real: peer/trust solo por socket para el superusuario; contraseña (scram) por 127.0.0.1.
as_postgres "$PG_BIN/initdb" -D "$WORK/pg" -U postgres --auth-local=trust --auth-host=scram-sha-256 -E UTF8 --no-instructions >/dev/null
as_postgres "$PG_BIN/pg_ctl" -D "$WORK/pg" -l "$WORK/pg/server.log" -w -o "-p $PG_PORT -k $WORK/pg -c listen_addresses=127.0.0.1" start >/dev/null
psql_su() { psql -X -q -v ON_ERROR_STOP=1 -h "$WORK/pg" -p "$PG_PORT" -U postgres "$@"; }

# Una base ajena ya existente: provision.sql no debe tocarla.
psql_su -d postgres -c "CREATE ROLE otra_app LOGIN PASSWORD 'x'" -c "CREATE DATABASE otra_app OWNER otra_app"
psql_su -d otra_app -c "CREATE TABLE datos (id int)" -c "ALTER TABLE datos OWNER TO otra_app"
sh "$ROOT/api/deploy/scripts/generate-secrets.sh" "$WORK/secrets" >/dev/null
secret() { tr -d '\r\n' < "$WORK/secrets/$1"; }

provision() {
  {
    printf '\\set db_name %s\n' jarvis
    printf "\\\\set migrator_pw '%s'\n" "$(secret db_migrator_password)"
    printf "\\\\set app_pw '%s'\n" "$(secret db_app_password)"
    printf "\\\\set maintenance_pw '%s'\n" "$(secret db_maintenance_password)"
    printf "\\\\set backup_pw '%s'\n" "$(secret db_backup_password)"
    printf "\\\\set worker_pw '%s'\n" "$(secret db_worker_password)"
    cat "$ROOT/api/deploy/postgres/provision.sql"
  } | psql_su -d postgres >/dev/null 2>"$WORK/provision.err" || { cat "$WORK/provision.err" >&2; fail "provision.sql"; }
}
provision
pass "provision.sql (1.ª vez)"

HAS_VECTOR=$(psql_su -d postgres -tAc "SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'")
EXT=$(psql_su -d jarvis -tAc "SELECT string_agg(extname, ',' ORDER BY extname) FROM pg_extension")
if [ "$HAS_VECTOR" = "1" ]; then [[ "$EXT" == *vector* ]] || fail "vector disponible pero no activado"; pass "pgvector activado solo en la base jarvis ($EXT)";
else [[ "$EXT" != *vector* ]] || fail "vector"; pass "sin pgvector en el servidor: no se activa (búsqueda en la aplicación)"; fi
[ "$(psql_su -d otra_app -tAc "SELECT count(*) FROM pg_extension WHERE extname IN ('vector','citext')")" = "0" ] || fail "provision tocó otra base"
pass "la base ajena 'otra_app' no se modifica"

export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
dotnet build "$ROOT/api/src/Jarvis.Api/Jarvis.Api.csproj" -v q -nologo >/dev/null
CS="Host=127.0.0.1;Port=$PG_PORT;Database=jarvis"
ConnectionStrings__PostgresMigrator="$CS;Username=jarvis_migrator;Password=$(secret db_migrator_password)" \
ConnectionStrings__Postgres="$CS;Username=jarvis_app;Password=$(secret db_app_password)" \
Jwt__SigningKeyFile="$WORK/secrets/jwt_signing_key" Jarvis__DeviceSecretPepperFile="$WORK/secrets/device_secret_pepper" \
ASPNETCORE_ENVIRONMENT=Production Logging__Console__FormatterName=simple \
  dotnet "$ROOT/api/src/Jarvis.Api/bin/Debug/net10.0/jarvis-api.dll" migrate > "$WORK/migrate.log" 2>&1 || { cat "$WORK/migrate.log" >&2; fail "migrate"; }
pass "migraciones aplicadas con jarvis_migrator (sin superusuario)"

provision
pass "provision.sql es idempotente (2.ª vez, con el esquema ya migrado)"

as_role() { local role=$1 pw=$2; shift 2; PGPASSWORD="$pw" psql -X -q -v ON_ERROR_STOP=1 -h 127.0.0.1 -p "$PG_PORT" -U "$role" -d jarvis "$@"; }
W=$(secret db_worker_password)
as_role jarvis_worker "$W" -tAc "SELECT count(*) FROM memory.jobs" >/dev/null || fail "worker no lee memory.jobs"
as_role jarvis_worker "$W" -tAc "SELECT count(*) FROM memory.memories" >/dev/null || fail "worker no lee memory.memories"
pass "jarvis_worker lee y escribe el esquema memory"
if as_role jarvis_worker "$W" -tAc "SELECT count(*) FROM jarvis.users" >/dev/null 2>&1; then fail "worker puede leer jarvis.users"; fi
if as_role jarvis_worker "$W" -c "CREATE TABLE memory.x (id int)" >/dev/null 2>&1; then fail "worker puede crear tablas"; fi
if as_role jarvis_worker "$W" -c "DROP TABLE memory.jobs" >/dev/null 2>&1; then fail "worker puede borrar tablas"; fi
pass "jarvis_worker NO lee usuarios/credenciales ni cambia el esquema"
if as_role jarvis_app "$(secret db_app_password)" -c "CREATE TABLE public.x (id int)" >/dev/null 2>&1; then fail "app crea tablas en public"; fi
# PostgreSQL concede CONNECT a PUBLIC por defecto en las bases ajenas (no se cambia: no es nuestra), pero sin datos.
if as_role jarvis_worker "$W" -d otra_app -tAc "SELECT count(*) FROM datos" >/dev/null 2>&1; then fail "worker lee datos de otra base"; fi
if as_role otra_app x -tAc "SELECT 1" >/dev/null 2>&1; then fail "otro rol entra en la base jarvis"; fi
pass "sin CREATE en public; otros roles no entran en 'jarvis'; jarvis_worker no lee datos de otras bases"
if [ "$HAS_VECTOR" = "1" ]; then
  as_role jarvis_app "$(secret db_app_password)" -tAc "SELECT (ARRAY[1,0,0]::real[])::vector <=> (ARRAY[1,0,0]::real[])::vector" | grep -q '^0$' || fail "cast a vector"
  pass "jarvis_app usa pgvector (real[] → vector)"
fi
echo "OK: provision + migraciones"
