# EPIC-11 — Deployment

DoD común: [user-stories.md](../user-stories.md#definition-of-done-común-aplica-a-todas-las-hu).

Referencia central: [Deployment Architecture](../../06-infrastructure/deployment-architecture.md), [Docker Strategy](../../06-infrastructure/docker-strategy.md), [Nginx](../../06-infrastructure/nginx.md), [Networking](../../06-infrastructure/networking.md).

---

## HU-051 — Imagen Docker de la API

- **Objetivo:** imagen de producción mínima y no-root.
- **Descripción:** Dockerfile multi-stage (`sdk:10.0` → `aspnet:10.0-noble-chiseled`), usuario no-root, subcomandos `migrate`, `bootstrap-owner`, `healthcheck`; etiquetas semver + git sha.
- **Criterios de aceptación:**
  1. Imagen lo más pequeña posible (meta orientativa 150 MB; la implementación mide 195 MB por EF Core/Npgsql/OpenTelemetry); corre con `read_only: true` y `cap_drop: ALL`.
  2. `docker run … healthcheck` funciona sin shell.
  3. Escaneo de vulnerabilidades (p. ej. `trivy`) sin críticas.
- **Dependencias:** HU-006.
- **Consideraciones técnicas:** `.dockerignore` excluye secretos y artefactos.
- **Pruebas esperadas:** build y arranque en CI.
- **Definition of Done:** DoD común.

---

## HU-052 — Nginx, TLS y Let's Encrypt

- **Objetivo:** configuración de nginx y certbot según [Nginx](../../06-infrastructure/nginx.md).
- **Descripción:** archivos de configuración versionados (con `${JARVIS_DOMAIN}` como plantilla); procedimiento de emisión inicial; deploy-hook que recarga nginx, copia certificados para Mosquitto y lo reinicia/recarga.
- **Criterios de aceptación:**
  1. SSL Labs (o `testssl.sh`) calificación A o superior.
  2. `/health/live` accesible; `/health/ready` → 404 desde fuera.
  3. SignalR funciona por WSS a través de nginx.
  4. Rate limits de `/api/v1/auth/` verificados (429).
  5. Logs de nginx sin query strings de `/hubs/`.
  6. Renovación simulada (`certbot renew --dry-run`) exitosa y hook ejecutado.
- **Dependencias:** HU-051.
- **Consideraciones técnicas:** Mosquitto usa copia de certificados con permisos para su UID.
- **Pruebas esperadas:** verificación manual guiada + comandos documentados.
- **Definition of Done:** DoD común + runbook en `docs/06-infrastructure/nginx.md` actualizado con comandos exactos.

---

## HU-053 — Docker Compose de producción

- **Objetivo:** `docker-compose.yml` según [Docker Strategy](../../06-infrastructure/docker-strategy.md).
- **Descripción:** servicios `nginx`, `jarvis-api`, `postgres`, `mosquitto`; redes `edge`, `backend` (internal), `mqtt-public`; secrets de archivo; health checks; `restart: unless-stopped`; límites de recursos; hardening; logging con rotación; `.env` sin secretos; script de init de roles PostgreSQL.
- **Criterios de aceptación:**
  1. `docker compose config` válido; solo 80, 443 y 8883 publicados.
  2. PostgreSQL inalcanzable desde fuera del host (nmap) y desde nginx.
  3. `docker inspect` no muestra valores de secretos en variables de entorno.
  4. Reinicio del host → todo vuelve a `healthy` sin intervención.
- **Dependencias:** HU-051, HU-052, HU-020.
- **Consideraciones técnicas:** imágenes fijadas por digest.
- **Pruebas esperadas:** despliegue en una VM de prueba o en el VPS.
- **Definition of Done:** DoD común.

---

## HU-054 — Provisión y hardening del VPS

- **Objetivo:** host listo y seguro.
- **Descripción:** runbook: Ubuntu 24.04, usuario `jarvis`, SSH solo llave, `PermitRootLogin no`, ufw (22 restringido, 80, 443, 8883), fail2ban, unattended-upgrades, Docker Engine + Compose v2, zona horaria UTC, `/opt/jarvis` con permisos, DNS del dominio.
- **Criterios de aceptación:**
  1. Login por contraseña SSH deshabilitado.
  2. `nmap` externo muestra solo 22/80/443/8883.
  3. Actualizaciones automáticas activas.
- **Dependencias:** ninguna (puede hacerse en paralelo desde el inicio).
- **Consideraciones técnicas:** documentar el proveedor elegido y su firewall de red si existe (doble barrera).
- **Pruebas esperadas:** checklist verificada.
- **Definition of Done:** runbook en `docs/06-infrastructure/` y checklist completada.

---

## HU-055 — Backups y restauración

- **Objetivo:** cumplir [Backup & Recovery](../../06-infrastructure/backup-recovery.md).
- **Descripción:** job systemd diario: `pg_dump -Fc` (rol `jarvis_backup`), copia de `dynamic-security.json`, cifrado `age`, subida a almacenamiento externo con credencial de solo escritura, rotación local; runbook de restauración.
- **Criterios de aceptación:**
  1. Backup diario generado y subido; alerta si falta (HU-056).
  2. **Restauración de prueba completa** en entorno aislado, con conexión exitosa de un simulador usando su credencial original.
  3. La clave privada de `age` no está en el VPS.
- **Dependencias:** HU-053.
- **Consideraciones técnicas:** proveedor de almacenamiento: decisión pendiente.
- **Pruebas esperadas:** ejercicio de restauración documentado con fecha y resultado.
- **Definition of Done:** DoD común.

---

## HU-056 — Monitoreo y alertas mínimas

- **Objetivo:** enterarse de caídas sin stack de monitoreo completo.
- **Descripción:** uptime check externo a `/health/live` y al puerto 8883; script cron en el host para disco, expiración de certificado y antigüedad del último backup ([Observability](../../06-infrastructure/observability.md#alertas-mvp-mínimo)); notificación por el canal decidido.
- **Criterios de aceptación:**
  1. Detener la API dispara alerta en < 5 min.
  2. Simular disco > 80 % dispara alerta.
- **Dependencias:** HU-053.
- **Consideraciones técnicas:** canal de alertas: decisión pendiente.
- **Pruebas esperadas:** pruebas manuales documentadas.
- **Definition of Done:** DoD común.
