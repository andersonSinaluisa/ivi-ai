#!/bin/sh
# Invocado por /etc/letsencrypt/renewal-hooks/deploy/jarvis.sh (instalado por deploy.sh) tras renovar el certificado:
# recarga el nginx NATIVO y entrega el certificado nuevo a Mosquitto (contenedor, uid 1883).
set -eu
DOMAIN=${JARVIS_DOMAIN:?JARVIS_DOMAIN required}
CERTS=${CONFIG_DIR:-/opt/jarvis/config}/mosquitto/certs
DEPLOY_DIR=${DEPLOY_DIR:?DEPLOY_DIR required}

install -d -m 0750 -o 1883 -g 1883 "$CERTS"
install -m 0400 -o 1883 -g 1883 "/etc/letsencrypt/live/$DOMAIN/fullchain.pem" "$CERTS/fullchain.pem"
install -m 0400 -o 1883 -g 1883 "/etc/letsencrypt/live/$DOMAIN/privkey.pem" "$CERTS/privkey.pem"

nginx -t && systemctl reload nginx
docker compose --project-directory "$DEPLOY_DIR" -f "$DEPLOY_DIR/docker-compose.yml" --env-file "$DEPLOY_DIR/.env" restart mosquitto
