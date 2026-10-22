#!/usr/bin/env bash
# Despliegue de Jarvis Cloud Core en un servidor con PostgreSQL y nginx NATIVOS (ADR-011).
# Docker solo ejecuta jarvis-api, jarvis-worker y mosquitto. Idempotente: sirve para el primer despliegue y para actualizaciones.
#
# Uso (como root o con sudo, desde cualquier directorio):
#   sudo ./deploy/scripts/deploy.sh [--owner-email <email>] [--skip-build] [--skip-certbot] [--skip-nginx]
#
# Requiere deploy/.env (ver deploy/.env.example) con al menos JARVIS_DOMAIN y CERTBOT_EMAIL.
#
# Pasos:
#   1. Requisitos (docker, compose, psql >= 15, nginx, certbot, openssl)
#   2. Secretos en $SECRETS_DIR (no sobrescribe los existentes)
#   3. PostgreSQL nativo: roles, base dedicada, esquemas jarvis/memory, citext, pgvector si está instalado (provision.sql)
#      No toca otras bases ni la configuración global de PostgreSQL.
#   4. Certificado Let's Encrypt (webroot, vía el nginx del host) + copia para Mosquitto
#   5. Mosquitto (Docker) + bootstrap de Dynamic Security (solo la primera vez)
#   6. Imágenes de la API y del worker + migraciones (rol jarvis_migrator; solo aditivas, ver runbooks.md)
#   7. jarvis-api (Docker, red del host, 127.0.0.1:8080) + readiness; jarvis-worker + healthcheck
#   8. Sitio nginx nativo (nginx -t antes de recargar)
#   9. Hook de renovación de certbot
#  10. Owner inicial (opcional, contraseña por teclado)
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$DEPLOY_DIR/.env"
OWNER_EMAIL=""
SKIP_BUILD=0
SKIP_CERTBOT=0
SKIP_NGINX=0

while [ $# -gt 0 ]; do
  case "$1" in
    --owner-email) OWNER_EMAIL="${2:?--owner-email requires a value}"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --skip-certbot) SKIP_CERTBOT=1; shift ;;
    --skip-nginx) SKIP_NGINX=1; shift ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

log() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
ok() { printf '    \033[0;32m✓\033[0m %s\n' "$*"; }
warn() { printf '    \033[0;33m!\033[0m %s\n' "$*"; }
die() { printf '\n\033[0;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "Run as root (sudo): needs PostgreSQL superuser (peer), nginx and /etc/letsencrypt."
[ -f "$ENV_FILE" ] || die "Missing $ENV_FILE. Copy deploy/.env.example and set JARVIS_DOMAIN and CERTBOT_EMAIL."

set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
: "${JARVIS_DOMAIN:?JARVIS_DOMAIN is required in deploy/.env}"
SECRETS_DIR="${SECRETS_DIR:-/opt/jarvis/secrets}"
CONFIG_DIR="${CONFIG_DIR:-/opt/jarvis/config}"
STATE_DIR="${STATE_DIR:-/opt/jarvis/state}"
PG_PORT="${PG_PORT:-5432}"
PG_DATABASE="${PG_DATABASE:-jarvis}"
CERTBOT_WEBROOT="${CERTBOT_WEBROOT:-/var/www/certbot}"
export SECRETS_DIR CONFIG_DIR PG_PORT PG_DATABASE JARVIS_DOMAIN

compose() { docker compose --project-directory "$DEPLOY_DIR" -f "$DEPLOY_DIR/docker-compose.yml" --env-file "$ENV_FILE" "$@"; }
psql_su() { sudo -u postgres psql -X -q -v ON_ERROR_STOP=1 -p "$PG_PORT" "$@"; }
secret() { tr -d '\r\n' < "$SECRETS_DIR/$1"; }

# ---------------------------------------------------------------------------------------------------------------------
log "1/10 Requisitos"
for cmd in docker psql nginx openssl sudo curl; do
  command -v "$cmd" >/dev/null 2>&1 || die "'$cmd' not found."
done
docker compose version >/dev/null 2>&1 || die "Docker Compose v2 plugin not found."
[ "$SKIP_CERTBOT" -eq 1 ] || command -v certbot >/dev/null 2>&1 || die "'certbot' not found (or use --skip-certbot)."
PG_VERSION_NUM=$(psql_su -d postgres -tAc 'SHOW server_version_num' | tr -d '[:space:]') || die "Cannot connect to local PostgreSQL as 'postgres' on port $PG_PORT."
[ "$PG_VERSION_NUM" -ge 150000 ] || die "PostgreSQL >= 15 required (found $PG_VERSION_NUM): the schema uses NULLS NOT DISTINCT."
ok "docker $(docker version --format '{{.Server.Version}}'), PostgreSQL $PG_VERSION_NUM, $(nginx -v 2>&1)"
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
  ufw status | grep -qE '^8883(/tcp)?\s+ALLOW' || warn "ufw is active but 8883/tcp is not allowed (devices need it): ufw allow 8883/tcp"
fi

# ---------------------------------------------------------------------------------------------------------------------
log "2/10 Secretos en $SECRETS_DIR"
sh "$DEPLOY_DIR/scripts/generate-secrets.sh" "$SECRETS_DIR" | sed 's/^/    /'
install -d -m 0700 "$CONFIG_DIR" "$STATE_DIR"
# Docker Compose monta los secretos de archivo con los permisos del host, y las imágenes .NET corren como UID/GID 1654
# (no root): los que leen la API y el worker pasan a root:1654 0440. El resto sigue 0400 root.
APP_GID="${BACKUP_READER_GID:-1654}"
for s in db_app_password db_migrator_password db_maintenance_password db_worker_password jwt_signing_key device_secret_pepper mqtt_cloud_password; do
  chgrp "$APP_GID" "$SECRETS_DIR/$s"
  chmod 0440 "$SECRETS_DIR/$s"
done
ok "secretos de la API y el worker legibles solo por root y el grupo $APP_GID"

# ---------------------------------------------------------------------------------------------------------------------
log "3/10 PostgreSQL nativo (base '$PG_DATABASE')"
# Las contraseñas viajan por stdin (variables psql), nunca como argumentos visibles en `ps`.
if ! {
  printf '\\set db_name %s\n' "$PG_DATABASE"
  printf "\\\\set migrator_pw '%s'\n" "$(secret db_migrator_password)"
  printf "\\\\set app_pw '%s'\n" "$(secret db_app_password)"
  printf "\\\\set maintenance_pw '%s'\n" "$(secret db_maintenance_password)"
  printf "\\\\set backup_pw '%s'\n" "$(secret db_backup_password)"
  printf "\\\\set worker_pw '%s'\n" "$(secret db_worker_password)"
  cat "$DEPLOY_DIR/postgres/provision.sql"
} | psql_su -d postgres > "$STATE_DIR/.provision.log" 2>&1; then
  cat "$STATE_DIR/.provision.log" >&2
  die "PostgreSQL provisioning failed."
fi
grep -v -E 'NOTICE:.*(already|skipping)' "$STATE_DIR/.provision.log" | sed 's/^/    /' || true
psql_su -d "$PG_DATABASE" -tAc "SELECT 1 FROM pg_namespace WHERE nspname = 'jarvis'" | grep -q 1 || die "PostgreSQL provisioning failed."
PGPASSWORD="$(secret db_app_password)" psql -X -h 127.0.0.1 -p "$PG_PORT" -U jarvis_app -d "$PG_DATABASE" -tAc 'SELECT 1' >/dev/null 2>&1 \
  || die "jarvis_app cannot log in over 127.0.0.1:$PG_PORT with a password. Check pg_hba.conf has 'host $PG_DATABASE all 127.0.0.1/32 scram-sha-256' (or md5) and reload PostgreSQL."
PGPASSWORD="$(secret db_worker_password)" psql -X -h 127.0.0.1 -p "$PG_PORT" -U jarvis_worker -d "$PG_DATABASE" -tAc 'SELECT 1' >/dev/null 2>&1 \
  || die "jarvis_worker cannot log in over 127.0.0.1:$PG_PORT (same pg_hba.conf rule as jarvis_app)."
if psql_su -d "$PG_DATABASE" -tAc "SELECT 1 FROM pg_extension WHERE extname = 'vector'" | grep -q 1; then
  ok "pgvector activo en '$PG_DATABASE' (búsqueda por similitud en PostgreSQL)"
else
  warn "pgvector no está instalado en el servidor: la búsqueda por similitud se hará en la aplicación (ADR-014). Opcional: apt install postgresql-<versión>-pgvector y volver a ejecutar deploy.sh"
fi
ok "roles jarvis_migrator/app/maintenance/backup/worker, schemas jarvis y memory, citext; password login over 127.0.0.1 works"

# ---------------------------------------------------------------------------------------------------------------------
LIVE_DIR="/etc/letsencrypt/live/$JARVIS_DOMAIN"
if [ -d /etc/nginx/sites-available ] && [ -d /etc/nginx/sites-enabled ]; then
  NGINX_SITE="/etc/nginx/sites-available/jarvis.conf"
  NGINX_LINK="/etc/nginx/sites-enabled/jarvis.conf"
else
  NGINX_SITE="/etc/nginx/conf.d/jarvis.conf"
  NGINX_LINK=""
fi

nginx_at_least() { # $1 = versión mínima
  local current
  current=$(nginx -v 2>&1 | sed -n 's#.*nginx/\([0-9.]*\).*#\1#p')
  [ "$(printf '%s\n%s\n' "$1" "$current" | sort -V | head -1)" = "$1" ]
}

install_nginx_site() { # $1 = full|http
  local tmp
  tmp=$(mktemp)
  if [ "$1" = "http" ]; then
    # Solo el server :80 (desafío ACME) mientras no existe el certificado.
    cat > "$tmp" <<EOF
server {
  listen 80;
  listen [::]:80;
  server_name $JARVIS_DOMAIN;
  location /.well-known/acme-challenge/ { root $CERTBOT_WEBROOT; }
  location / { return 404; }
}
EOF
  else
    sed "s/__JARVIS_DOMAIN__/$JARVIS_DOMAIN/g" "$DEPLOY_DIR/nginx/jarvis-site.conf.template" > "$tmp"
    # `http2 on;` existe desde nginx 1.25.1; Ubuntu 22.04/24.04 traen 1.18/1.24: ahí va en `listen ... ssl http2`.
    if ! nginx_at_least 1.25.1; then
      sed -i -e '/^[[:space:]]*http2 on;/d' -e 's/^\([[:space:]]*listen [^;]*443 ssl\);/\1 http2;/' "$tmp"
    fi
  fi
  [ -f "$NGINX_SITE" ] && cp "$NGINX_SITE" "$NGINX_SITE.bak"
  install -m 0644 "$tmp" "$NGINX_SITE"
  rm -f "$tmp"
  [ -n "$NGINX_LINK" ] && ln -sf "$NGINX_SITE" "$NGINX_LINK"
  if ! nginx -t 2>/tmp/jarvis-nginx-test.log; then
    cat /tmp/jarvis-nginx-test.log >&2
    [ -f "$NGINX_SITE.bak" ] && mv "$NGINX_SITE.bak" "$NGINX_SITE" || rm -f "$NGINX_SITE" "$NGINX_LINK"
    die "nginx -t failed; previous configuration restored."
  fi
  systemctl reload nginx
}

log "4/10 Certificado TLS para $JARVIS_DOMAIN"
install -d -m 0755 "$CERTBOT_WEBROOT"
if [ "$SKIP_CERTBOT" -eq 0 ] && [ ! -f "$LIVE_DIR/fullchain.pem" ]; then
  : "${CERTBOT_EMAIL:?CERTBOT_EMAIL is required in deploy/.env to obtain the certificate}"
  install_nginx_site http
  certbot certonly --webroot -w "$CERTBOT_WEBROOT" -d "$JARVIS_DOMAIN" --email "$CERTBOT_EMAIL" --agree-tos --no-eff-email --non-interactive
fi
[ -f "$LIVE_DIR/fullchain.pem" ] || die "No certificate at $LIVE_DIR (obtain it or run without --skip-certbot)."
MOSQ_CERTS="$CONFIG_DIR/mosquitto/certs"
install -d -m 0750 -o 1883 -g 1883 "$MOSQ_CERTS"
install -m 0400 -o 1883 -g 1883 "$LIVE_DIR/fullchain.pem" "$MOSQ_CERTS/fullchain.pem"
install -m 0400 -o 1883 -g 1883 "$LIVE_DIR/privkey.pem" "$MOSQ_CERTS/privkey.pem"
ok "certificate valid until $(openssl x509 -enddate -noout -in "$LIVE_DIR/fullchain.pem" | cut -d= -f2); copied for Mosquitto (uid 1883)"

# ---------------------------------------------------------------------------------------------------------------------
log "5/10 Mosquitto"
compose up -d mosquitto
for _ in $(seq 1 30); do
  [ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q mosquitto)")" = "healthy" ] && break
  sleep 2
done
[ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q mosquitto)")" = "healthy" ] || die "Mosquitto is not healthy: docker compose logs mosquitto"
ok "mosquitto healthy (8883 public TLS, 1883 on 127.0.0.1)"

# ---------------------------------------------------------------------------------------------------------------------
log "6/10 Imagen y migraciones"
if [ "$SKIP_BUILD" -eq 0 ]; then
  compose build jarvis-api jarvis-worker
fi
if [ ! -f "$STATE_DIR/broker-bootstrapped" ]; then
  compose run --rm --no-deps \
    -v "$SECRETS_DIR/mosquitto_dynsec_admin_password:/run/secrets/dynsec_admin:ro" \
    jarvis-api broker-bootstrap --host 127.0.0.1 --port 1883 --admin-user bootstrap-admin \
    --admin-password-file /run/secrets/dynsec_admin --cloud-password-file /run/secrets/mqtt_cloud_password --delete-admin
  touch "$STATE_DIR/broker-bootstrapped"
  ok "Dynamic Security initialized (roles, default deny, jarvis-cloud); temporary admin deleted"
else
  ok "Dynamic Security already initialized ($STATE_DIR/broker-bootstrapped)"
fi
compose run --rm --no-deps jarvis-api migrate | sed 's/^/    /'

# ---------------------------------------------------------------------------------------------------------------------
log "7/10 jarvis-api"
compose up -d jarvis-api
for _ in $(seq 1 45); do
  if curl -fsS --max-time 3 http://127.0.0.1:8080/health/ready >/tmp/jarvis-ready.json 2>/dev/null; then break; fi
  sleep 2
done
curl -fsS --max-time 3 http://127.0.0.1:8080/health/ready >/tmp/jarvis-ready.json 2>/dev/null || die "jarvis-api not ready: docker compose logs jarvis-api"
ok "ready: $(cat /tmp/jarvis-ready.json)"
install -d -m 0750 -g "$APP_GID" "${BACKUP_DIR:-/opt/jarvis/backups}"
compose up -d jarvis-worker
for _ in $(seq 1 45); do
  [ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q jarvis-worker)")" = "healthy" ] && break
  sleep 2
done
[ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q jarvis-worker)")" = "healthy" ] || die "jarvis-worker is not healthy: docker compose logs jarvis-worker"
ok "jarvis-worker healthy (sin puertos; backups montados en solo lectura)"

# ---------------------------------------------------------------------------------------------------------------------
log "8/10 nginx (sitio $NGINX_SITE)"
if [ "$SKIP_NGINX" -eq 0 ]; then
  install_nginx_site full
  curl -fsS --max-time 5 --resolve "$JARVIS_DOMAIN:443:127.0.0.1" "https://$JARVIS_DOMAIN/health/live" >/dev/null \
    && ok "https://$JARVIS_DOMAIN/health/live responde vía nginx" \
    || warn "nginx instalado pero /health/live no respondió por HTTPS local; revisar /var/log/nginx/jarvis.error.log"
else
  warn "--skip-nginx: sitio no instalado"
fi

# ---------------------------------------------------------------------------------------------------------------------
log "9/10 Renovación de certificados"
HOOK=/etc/letsencrypt/renewal-hooks/deploy/jarvis.sh
install -d /etc/letsencrypt/renewal-hooks/deploy
cat > "$HOOK" <<EOF
#!/bin/sh
# Generado por deploy.sh: tras renovar $JARVIS_DOMAIN recarga nginx y entrega el certificado a Mosquitto.
case "\${RENEWED_LINEAGE:-}" in *"/$JARVIS_DOMAIN") ;; *) exit 0 ;; esac
JARVIS_DOMAIN="$JARVIS_DOMAIN" CONFIG_DIR="$CONFIG_DIR" DEPLOY_DIR="$DEPLOY_DIR" exec "$DEPLOY_DIR/scripts/certbot-deploy-hook.sh"
EOF
chmod 0755 "$HOOK"
ok "hook instalado en $HOOK (probar con: certbot renew --dry-run)"

# ---------------------------------------------------------------------------------------------------------------------
log "10/10 Owner inicial"
if [ -n "$OWNER_EMAIL" ]; then
  read -r -s -p "    Password for $OWNER_EMAIL (min 12 chars): " OWNER_PASSWORD; echo
  printf '%s\n' "$OWNER_PASSWORD" | compose run --rm --no-deps -T jarvis-api bootstrap-owner --email "$OWNER_EMAIL" --display-name Owner
  unset OWNER_PASSWORD
else
  ok "omitido (usar --owner-email <email> para crear el primer owner)"
fi

log "Despliegue completo"
echo "    API:   https://$JARVIS_DOMAIN/api/v1   (local: http://127.0.0.1:8080)"
echo "    MQTT:  mqtts://$JARVIS_DOMAIN:8883"
echo "    Logs:  docker compose -f $DEPLOY_DIR/docker-compose.yml logs -f jarvis-api"
