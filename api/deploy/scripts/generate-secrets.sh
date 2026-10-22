#!/bin/sh
# Genera los archivos de secretos (docs/05-security/secrets-management.md).
# Uso: generate-secrets.sh <directorio>   (producción: /opt/jarvis/secrets, desarrollo: .local-secrets)
# postgres_password solo lo usa docker-compose.dev.yml; en producción PostgreSQL es nativo (peer auth).
# Nunca sobrescribe un secreto existente.
set -eu
DIR=${1:?target directory required}
umask 077
mkdir -p "$DIR"
chmod 700 "$DIR"

gen() { # nombre bytes
  if [ ! -f "$DIR/$1" ]; then
    openssl rand -base64 "$2" | tr -d '\r\n=' | tr '+/' '-_' > "$DIR/$1"
    chmod 400 "$DIR/$1"
    echo "created $1"
  fi
}

gen postgres_password 32
gen db_app_password 32
gen db_migrator_password 32
gen db_maintenance_password 32
gen db_backup_password 32
gen db_worker_password 32
gen jwt_signing_key 48
gen device_secret_pepper 48
gen mqtt_cloud_password 32
gen mosquitto_dynsec_admin_password 32
echo "secrets ready in $DIR"
