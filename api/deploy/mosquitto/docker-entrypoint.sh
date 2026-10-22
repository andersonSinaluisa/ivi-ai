#!/bin/sh
# Inicializa Dynamic Security la primera vez con un admin TEMPORAL; luego `jarvis-api broker-bootstrap`
# crea roles y el client jarvis-cloud y elimina ese admin (docs/05-security/secrets-management.md).
set -eu
DYNSEC_FILE=/mosquitto/data/dynamic-security.json
CONF=${MOSQUITTO_CONF:-/mosquitto/config/mosquitto.conf}

if [ ! -f "$DYNSEC_FILE" ]; then
  if [ -z "${DYNSEC_ADMIN_PASSWORD_FILE:-}" ] || [ ! -f "$DYNSEC_ADMIN_PASSWORD_FILE" ]; then
    echo "dynamic-security.json missing and DYNSEC_ADMIN_PASSWORD_FILE not provided" >&2
    exit 1
  fi
  # tr elimina saltos de línea (incluido \r de archivos generados en Windows).
  ADMIN_PASSWORD=$(tr -d '\r\n' < "$DYNSEC_ADMIN_PASSWORD_FILE")
  mosquitto_ctrl dynsec init "$DYNSEC_FILE" "${DYNSEC_ADMIN_USER:-bootstrap-admin}" "$ADMIN_PASSWORD"
fi

chown -R mosquitto:mosquitto /mosquitto/data
chmod 700 /mosquitto/data
chmod 600 "$DYNSEC_FILE"
exec /usr/sbin/mosquitto -c "$CONF"
