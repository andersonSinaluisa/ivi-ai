# Runbooks de operación

Procedimientos para el servidor de producción según [ADR-011](../08-decisions/ADR-011-native-postgres-nginx.md): **PostgreSQL y nginx nativos** en el host; Docker solo ejecuta `jarvis-api` y `mosquitto`. Todo el despliegue lo hace un único script idempotente: [`deploy/scripts/deploy.sh`](../../deploy/scripts/deploy.sh).

## 1. Requisitos del servidor

| Requisito | Notas |
|---|---|
| Linux con systemd (Ubuntu/Debian recomendados) | El script usa `systemctl reload nginx` |
| PostgreSQL **≥ 15**, nativo | Superusuario local `postgres` accesible por *peer* (`sudo -u postgres psql`). `pg_hba.conf` debe permitir `host all all 127.0.0.1/32 scram-sha-256` (o `md5`); es el valor por defecto en Ubuntu/Debian |
| nginx nativo | Incluye `sites-enabled/*` o `conf.d/*` dentro de `http {}` (configuración por defecto) |
| Docker Engine + Compose v2 | Para `jarvis-api` y `mosquitto` |
| certbot, openssl, curl | certbot en modo webroot |
| DNS | `A`/`AAAA` de `${JARVIS_DOMAIN}` → IP del servidor |
| Firewall | Entrantes: 22 (restringido), 80, 443, **8883**. PostgreSQL (5432), 1883, 8080 y 8081 **nunca** abiertos (la API y Mosquitto 1883 escuchan solo en 127.0.0.1) |

Hardening recomendado (HU-054): SSH solo con llave (`PasswordAuthentication no`, `PermitRootLogin no`), `fail2ban`, `unattended-upgrades`, zona horaria UTC.

```bash
sudo ufw allow 80/tcp && sudo ufw allow 443/tcp && sudo ufw allow 8883/tcp
```

## 2. Primer despliegue

```bash
# Monorepo: solo api/ (sparse checkout); /opt/jarvis/compose apunta a esa carpeta
sudo git clone --filter=blob:none --sparse <repo> /opt/jarvis/src && sudo git -C /opt/jarvis/src sparse-checkout set api
sudo ln -sfn /opt/jarvis/src/api /opt/jarvis/compose && cd /opt/jarvis/compose
sudo cp deploy/.env.example deploy/.env && sudo editor deploy/.env        # JARVIS_DOMAIN, CERTBOT_EMAIL, PG_PORT
sudo ./deploy/scripts/deploy.sh --owner-email owner@example.com
```

Qué hace `deploy.sh` (cada paso es seguro de repetir):

| Paso | Acción | Falla si |
|---|---|---|
| 1 | Verifica docker, compose, psql, nginx, certbot, openssl; PostgreSQL ≥ 15; avisa si ufw no permite 8883 | Falta algo o PostgreSQL < 15 |
| 2 | Genera los secretos que falten en `/opt/jarvis/secrets` (0400, nunca sobrescribe) | — |
| 3 | Ejecuta [`provision.sql`](../../deploy/postgres/provision.sql) como `postgres`: roles, base, esquema `jarvis`, `citext`; resincroniza contraseñas desde los secretos; comprueba login de `jarvis_app` por 127.0.0.1 | `pg_hba.conf` no permite contraseña por 127.0.0.1 |
| 4 | Si no hay certificado: instala un sitio nginx solo-HTTP con `/.well-known/acme-challenge/`, recarga nginx y ejecuta `certbot certonly --webroot`. Copia el certificado para Mosquitto (uid 1883, 0400) | DNS no apunta al servidor o el puerto 80 no es alcanzable |
| 5 | Arranca `mosquitto` (inicializa Dynamic Security con un admin temporal) | Mosquitto no llega a `healthy` |
| 6 | Construye la imagen; la primera vez ejecuta `broker-bootstrap --delete-admin` (marca en `/opt/jarvis/state/broker-bootstrapped`); ejecuta `migrate` con `jarvis_migrator` | Error de migración |
| 7 | Arranca `jarvis-api` y espera `http://127.0.0.1:8080/health/ready` | La API no queda lista |
| 8 | Instala el sitio completo de nginx, `nginx -t`, recarga y prueba `https://${JARVIS_DOMAIN}/health/live` | `nginx -t` falla (se restaura la configuración anterior) |
| 9 | Instala `/etc/letsencrypt/renewal-hooks/deploy/jarvis.sh` (recarga nginx y copia el certificado a Mosquitto) | — |
| 10 | Con `--owner-email`, pide la contraseña por teclado y crea el primer owner | Contraseña < 12 caracteres o común |

Opciones: `--skip-build` (reusar imagen), `--skip-certbot` (certificado gestionado por otro medio, debe existir en `/etc/letsencrypt/live/<dominio>/`), `--skip-nginx`.

Verificación final: [Acceptance Tests](../10-testing/acceptance-tests.md) AT-01 … AT-17 y `sudo certbot renew --dry-run`.

## 3. Actualización

```bash
sudo git -C /opt/jarvis/src pull && cd /opt/jarvis/compose
sudo ./deploy/scripts/deploy.sh
```

Las migraciones son compatibles hacia atrás (expand → migrate → contract) y en producción solo se aplican migraciones **aditivas**: una que borre o reescriba datos se ejecuta a mano, tras un backup verificado. Antes del primer despliegue en un servidor compartido: `deploy/scripts/inspect-vps.sh` (solo lectura) y [contabo-deployment.md](contabo-deployment.md). Rollback: `git checkout <tag anterior>` y repetir `deploy.sh`.

## 4. Backups

`deploy/scripts/backup.sh` ejecuta `pg_dump -Fc` **del host** con el rol `jarvis_backup` por 127.0.0.1, copia `dynamic-security.json` desde el contenedor de Mosquitto, cifra ambos con `age` (clave pública) y opcionalmente los sube con `rclone` a un remoto de solo escritura.

```bash
# Clave age: generar en una máquina de confianza; copiar solo la clave pública al servidor
age-keygen -o backup-key.txt && grep 'public key' backup-key.txt | cut -d' ' -f4 | sudo tee /opt/jarvis/backup-recipient.txt

sudo tee /etc/systemd/system/jarvis-backup.service >/dev/null <<'UNIT'
[Service]
Type=oneshot
Environment=REMOTE=jarvis-backups:jarvis
ExecStart=/opt/jarvis/compose/deploy/scripts/backup.sh
UNIT
sudo tee /etc/systemd/system/jarvis-backup.timer >/dev/null <<'UNIT'
[Timer]
OnCalendar=*-*-* 03:00:00 UTC
Persistent=true
[Install]
WantedBy=timers.target
UNIT
sudo systemctl enable --now jarvis-backup.timer
```

Si el PostgreSQL del host ya tiene su propia política de backup (p. ej. de todo el clúster), `backup.sh` sigue siendo necesario para `dynamic-security.json`; el dump de la base puede omitirse coordinándolo con esa política.

## 5. Restauración

1. Preparar el servidor (§1) y restaurar `/opt/jarvis/secrets` desde su copia cifrada.
2. Desplegar con la base vacía (roles, esquema y servicios):

   ```bash
   sudo -u postgres dropdb --if-exists jarvis          # SOLO en un servidor nuevo o si se restaura encima
   sudo ./deploy/scripts/deploy.sh --skip-build         # crea roles, base y esquema vacíos y arranca todo
   ```
3. Restaurar datos:
   ```bash
   age -d -i backup-key.txt jarvis-<stamp>.dump.age > jarvis.dump
   sudo docker compose -f deploy/docker-compose.yml --env-file deploy/.env stop jarvis-api
   sudo -u postgres pg_restore --clean --if-exists -d jarvis jarvis.dump
   ```
4. Restaurar `dynamic-security.json` en el volumen de Mosquitto (así los devices conservan sus credenciales):
   ```bash
   age -d -i backup-key.txt dynsec-<stamp>.json.age > dynamic-security.json
   sudo docker compose -f deploy/docker-compose.yml --env-file deploy/.env cp dynamic-security.json mosquitto:/mosquitto/data/dynamic-security.json
   sudo docker compose -f deploy/docker-compose.yml --env-file deploy/.env restart mosquitto jarvis-api
   ```
5. Verificar `/health/ready`, login, `GET /api/v1/devices` y la conexión de un simulador con su credencial original.

Sin backup de `dynamic-security.json`: borrar `/opt/jarvis/state/broker-bootstrapped` y el volumen `jarvis_mosquitto-data`, repetir `deploy.sh` y rotar la credencial de cada device (`brokerSynced = false` en la API los identifica).

Registrar fecha y resultado de cada simulacro (objetivo: trimestral).

## 6. Monitoreo

```bash
# cron de root, cada 5 minutos
*/5 * * * * ALERT_TO=owner@example.com JARVIS_DOMAIN=jarvis.example.com /opt/jarvis/compose/deploy/scripts/monitor.sh
```

Más un uptime check externo contra `https://${JARVIS_DOMAIN}/health/live` y el puerto 8883.

## 7. Operaciones frecuentes

| Operación | Cómo |
|---|---|
| Revocar un device robado | `POST /api/v1/devices/{id}/revoke` (desconexión inmediata en MQTT) |
| Rotar credencial de un device | `POST /api/v1/devices/{id}/credentials/rotate` y reprovisionar el device |
| Historia de una tarea | `GET /api/v1/events?taskId={id}&after=0` |
| Readiness / métricas | `curl http://127.0.0.1:8080/health/ready` · `curl http://127.0.0.1:8081/metrics` (solo desde el propio servidor) |
| Logs | `docker compose -f deploy/docker-compose.yml logs -f jarvis-api` · nginx: `/var/log/nginx/jarvis.access.log` |
| Crear otro owner | `docker compose -f deploy/docker-compose.yml --env-file deploy/.env run --rm -T jarvis-api bootstrap-owner --email <e>` (contraseña por stdin) |
| Contraseña del owner olvidada | `docker compose -f deploy/docker-compose.yml --env-file deploy/.env run --rm -T jarvis-api reset-owner-password --email <e>` (nueva contraseña por stdin; cierra sus sesiones y quita el bloqueo) |
