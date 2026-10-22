#!/bin/sh
# HU-055 (docs/06-infrastructure/backup-recovery.md). Ejecutar diario como root desde un timer systemd.
# PostgreSQL es NATIVO: pg_dump del host con el rol jarvis_backup por 127.0.0.1.
# Escribe SHA256SUMS para el job backup.verify de jarvis-worker (ADR-016).
# Requiere: age (clave PÚBLICA en $AGE_RECIPIENT_FILE); opcional rclone con un remoto de SOLO ESCRITURA ($REMOTE).
set -eu
DEPLOY_DIR="$(cd "$(dirname "$0")/.." && pwd)"
set -a; . "$DEPLOY_DIR/.env"; set +a
SECRETS_DIR=${SECRETS_DIR:-/opt/jarvis/secrets}
OUT=${BACKUP_DIR:-/opt/jarvis/backups}
AGE_RECIPIENT_FILE=${AGE_RECIPIENT_FILE:-/opt/jarvis/backup-recipient.txt}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
umask 077
mkdir -p "$OUT"

PGPASSWORD="$(tr -d '\r\n' < "$SECRETS_DIR/db_backup_password")" \
  pg_dump -Fc -h 127.0.0.1 -p "${PG_PORT:-5432}" -U jarvis_backup "${PG_DATABASE:-jarvis}" > "$OUT/jarvis-$STAMP.dump"
docker compose --project-directory "$DEPLOY_DIR" -f "$DEPLOY_DIR/docker-compose.yml" --env-file "$DEPLOY_DIR/.env" \
  exec -T mosquitto cat /mosquitto/data/dynamic-security.json > "$OUT/dynsec-$STAMP.json"

for f in "$OUT/jarvis-$STAMP.dump" "$OUT/dynsec-$STAMP.json"; do
  age -R "$AGE_RECIPIENT_FILE" -o "$f.age" "$f"
  rm -f "$f"
done

if [ -n "${REMOTE:-}" ]; then
  rclone copy "$OUT" "$REMOTE" --include "*-$STAMP.*.age"
fi

find "$OUT" -name '*.age' -mtime +7 -delete

# Sumas de comprobación de lo que queda (jarvis-worker las verifica con el job backup.verify, en solo lectura).
( cd "$OUT" && sha256sum -- *.age > SHA256SUMS.tmp && mv SHA256SUMS.tmp SHA256SUMS )
# Los .age están cifrados: el worker (grupo $BACKUP_READER_GID, sin la clave privada) solo puede leerlos.
chgrp "${BACKUP_READER_GID:-1654}" "$OUT" "$OUT"/*.age "$OUT/SHA256SUMS"
chmod 0750 "$OUT"
chmod 0440 "$OUT"/*.age "$OUT/SHA256SUMS"
touch "$OUT/.last-success"
chgrp "${BACKUP_READER_GID:-1654}" "$OUT/.last-success"
chmod 0440 "$OUT/.last-success"
echo "backup $STAMP done"
