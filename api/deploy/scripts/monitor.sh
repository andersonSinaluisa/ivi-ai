#!/bin/sh
# HU-056: alertas mínimas por cron (cada 5 min). Envía email con `mail` si algo falla.
set -u
ALERT_TO=${ALERT_TO:?ALERT_TO required}
DOMAIN=${JARVIS_DOMAIN:?JARVIS_DOMAIN required}
BACKUP_DIR=${BACKUP_DIR:-/opt/jarvis/backups}
problems=""

curl -fsS --max-time 5 "https://$DOMAIN/health/live" >/dev/null || problems="$problems\n- /health/live no responde"
timeout 5 sh -c "</dev/tcp/$DOMAIN/8883" 2>/dev/null || nc -z -w 5 "$DOMAIN" 8883 || problems="$problems\n- puerto 8883 no responde"

usage=$(df -P / | awk 'NR==2 {gsub("%","",$5); print $5}')
[ "$usage" -gt 80 ] && problems="$problems\n- disco al ${usage}%"

expiry=$(echo | openssl s_client -connect "$DOMAIN:443" -servername "$DOMAIN" 2>/dev/null | openssl x509 -noout -enddate | cut -d= -f2)
if [ -n "$expiry" ]; then
  days=$(( ( $(date -d "$expiry" +%s) - $(date +%s) ) / 86400 ))
  [ "$days" -lt 14 ] && problems="$problems\n- certificado expira en $days días"
fi

if [ ! -f "$BACKUP_DIR/.last-success" ] || [ $(( $(date +%s) - $(stat -c %Y "$BACKUP_DIR/.last-success") )) -gt 93600 ]; then
  problems="$problems\n- no hay backup exitoso en las últimas 26 h"
fi

if [ -n "$problems" ]; then
  printf "Jarvis Cloud (%s):%b\n" "$DOMAIN" "$problems" | mail -s "[jarvis] alerta" "$ALERT_TO"
fi
