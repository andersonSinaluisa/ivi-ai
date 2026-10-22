#!/usr/bin/env bash
# Inspección de SOLO LECTURA del VPS antes de desplegar Jarvis (Contabo u otro). No instala, no reinicia, no escribe
# nada fuera de la salida estándar: solo consulta. Pensado para ejecutarlo a mano y revisar el informe.
#
# Uso:   sudo ./deploy/scripts/inspect-vps.sh > informe-vps.txt
#        (sin sudo también funciona, pero no podrá leer pg_hba.conf ni las bases de PostgreSQL)
#
# El informe NO contiene secretos: no lee archivos de secretos, ni variables de entorno de contenedores, ni
# contraseñas. Revísalo igualmente antes de compartirlo (incluye nombres de contenedores, puertos y dominios).
set -uo pipefail

section() { printf '\n===== %s =====\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
run() { "$@" 2>&1 || true; }

section "Sistema"
run uname -srm
[ -r /etc/os-release ] && . /etc/os-release && echo "SO: ${PRETTY_NAME:-?}"
echo "CPU: $(nproc 2>/dev/null || echo ?) núcleos"
run free -h
run df -h / /var /opt 2>/dev/null
echo "Hora UTC: $(date -u +%FT%TZ)"
have timedatectl && run timedatectl show -p NTPSynchronized

section "Puertos en escucha (TCP)"
if have ss; then run ss -tlnH | awk '{print $4}' | sort -u; else echo "ss no disponible"; fi
echo
for p in 80 443 1883 5432 8080 8081 8883; do
  if have ss && ss -tlnH "sport = :$p" 2>/dev/null | grep -q .; then
    echo "  :$p OCUPADO ($(ss -tlnH "sport = :$p" | awk '{print $4}' | tr '\n' ' '))"
  else
    echo "  :$p libre"
  fi
done

section "Cortafuegos"
if have ufw; then run ufw status verbose; else echo "ufw no instalado"; fi
have nft && echo "nftables: $(nft list ruleset 2>/dev/null | grep -c . ) líneas de reglas (no se muestran)"

section "Docker"
if have docker; then
  run docker version --format 'Docker {{.Server.Version}} (API {{.Server.APIVersion}})'
  run docker compose version
  echo
  echo "Contenedores (nombre | imagen | estado | puertos publicados):"
  run docker ps -a --format '  {{.Names}} | {{.Image}} | {{.Status}} | {{.Ports}}'
  echo
  echo "Redes:"; run docker network ls --format '  {{.Name}} ({{.Driver}})'
  echo "Volúmenes: $(docker volume ls -q 2>/dev/null | wc -l)"
  echo "Proyectos compose:"; run docker compose ls
  echo "Espacio de Docker:"; run docker system df
else
  echo "docker no instalado"
fi

section "PostgreSQL"
PSQL_OK=0
if have psql; then
  run psql --version
  if [ "$(id -u)" -eq 0 ] && id postgres >/dev/null 2>&1; then
    q() { sudo -u postgres psql -X -tA -c "$1" 2>&1; }
    if q 'SELECT 1' | grep -q '^1$'; then
      PSQL_OK=1
      echo "Servidor: $(q 'SHOW server_version')"
      echo "Puerto: $(q 'SHOW port')   listen_addresses: $(q 'SHOW listen_addresses')"
      echo "password_encryption: $(q 'SHOW password_encryption')   max_connections: $(q 'SHOW max_connections')"
      echo "Uso de conexiones: $(q 'SELECT count(*) FROM pg_stat_activity')"
      echo "Bases (nombre | tamaño):"
      q "SELECT '  ' || datname || ' | ' || pg_size_pretty(pg_database_size(datname)) FROM pg_database WHERE NOT datistemplate ORDER BY 1"
      echo "Bases que cualquier rol puede abrir (CONNECT a PUBLIC):"
      q "SELECT '  ' || datname FROM pg_database d WHERE NOT datistemplate AND has_database_privilege('public', d.oid, 'CONNECT') ORDER BY 1"
      echo "Roles con LOGIN (sin atributos sensibles):"
      q "SELECT '  ' || rolname || CASE WHEN rolsuper THEN ' (superusuario)' ELSE '' END FROM pg_roles WHERE rolcanlogin ORDER BY 1"
      echo "Roles de Jarvis ya existentes:"
      q "SELECT '  ' || rolname FROM pg_roles WHERE rolname LIKE 'jarvis\\_%' ORDER BY 1"
      echo "pgvector disponible: $(q "SELECT coalesce((SELECT default_version FROM pg_available_extensions WHERE name = 'vector'), 'NO (opcional: apt install postgresql-<versión>-pgvector)')")"
      echo "citext disponible: $(q "SELECT coalesce((SELECT default_version FROM pg_available_extensions WHERE name = 'citext'), 'NO')")"
      HBA=$(q 'SHOW hba_file')
      echo "pg_hba.conf ($HBA), reglas activas:"
      [ -r "$HBA" ] && grep -vE '^\s*(#|$)' "$HBA" | sed 's/^/  /'
    else
      echo "No se pudo consultar como 'postgres' (¿PostgreSQL en un contenedor o con otra autenticación?)."
    fi
  else
    echo "Ejecuta con sudo para consultar el servidor local."
  fi
else
  echo "psql no instalado en el host"
fi
[ "$PSQL_OK" -eq 0 ] && have docker && docker ps --format '{{.Names}} {{.Image}}' 2>/dev/null | grep -iE 'postgres|pgvector' | sed 's/^/  PostgreSQL en contenedor: /'
if have ss; then
  PUB=$(ss -tlnH 'sport = :5432' 2>/dev/null | awk '{print $4}' | grep -vE '^(127\.|\[::1\])' || true)
  [ -n "$PUB" ] && echo "AVISO: PostgreSQL escucha fuera de loopback: $PUB (revisa el cortafuegos; Jarvis nunca lo publica)"
fi

section "nginx"
if have nginx; then
  run nginx -v
  if [ "$(id -u)" -eq 0 ]; then run nginx -t; fi
  echo "Sitios:"
  for d in /etc/nginx/sites-enabled /etc/nginx/conf.d; do
    [ -d "$d" ] && for f in "$d"/*; do [ -e "$f" ] && echo "  $f -> server_name: $(grep -hoE 'server_name[^;]+' "$f" 2>/dev/null | sed 's/server_name//' | tr -s ' ' | tr '\n' ' ')"; done
  done
  grep -rqs 'jarvis_' /etc/nginx/ && echo "  Ya hay configuración con prefijo jarvis_ (despliegue previo)."
else
  echo "nginx no instalado en el host"
  have docker && docker ps --format '{{.Names}} {{.Image}} {{.Ports}}' 2>/dev/null | grep -iE 'nginx|traefik|caddy|proxy' | sed 's/^/  Proxy en contenedor: /'
fi

section "Certificados (Let's Encrypt)"
if have certbot && [ "$(id -u)" -eq 0 ]; then
  run certbot certificates 2>/dev/null | grep -E 'Certificate Name|Domains|Expiry'
else
  [ -d /etc/letsencrypt/live ] && ls /etc/letsencrypt/live 2>/dev/null | sed 's/^/  /' || echo "certbot no disponible o sin permisos"
fi

section "Herramientas que usa el despliegue"
for c in docker psql pg_dump nginx certbot openssl curl age rclone sha256sum git; do
  if have "$c"; then echo "  $c: sí"; else echo "  $c: NO"; fi
done

section "Jarvis en este servidor"
for d in /opt/jarvis /opt/jarvis/secrets /opt/jarvis/config /opt/jarvis/state /opt/jarvis/backups; do
  if [ -d "$d" ]; then echo "  $d existe ($(stat -c '%U:%G %a' "$d" 2>/dev/null))"; else echo "  $d no existe"; fi
done
[ -d /opt/jarvis/secrets ] && echo "  secretos presentes: $(ls /opt/jarvis/secrets 2>/dev/null | wc -l) archivos (no se leen)"
[ -f /opt/jarvis/backups/.last-success ] && echo "  último backup: $(stat -c '%y' /opt/jarvis/backups/.last-success)"
have docker && docker ps -a --format '{{.Names}}' 2>/dev/null | grep -E '^jarvis' | sed 's/^/  contenedor: /'

section "Ollama (opcional; Jarvis NO lo instala)"
if have ollama; then run ollama --version; else echo "  no instalado"; fi
if have nvidia-smi; then run nvidia-smi --query-gpu=name,memory.total --format=csv,noheader; else echo "  sin GPU NVIDIA visible (Qwen en CPU sería lento)"; fi

printf '\nFin del informe (solo lectura; no se cambió nada).\n'
