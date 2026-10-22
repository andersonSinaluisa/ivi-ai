# ADR-011 — PostgreSQL y nginx nativos en el host de producción

- **Status:** Accepted
- **Fecha:** 2026-09-19
- **Modifica:** el despliegue descrito en [Deployment Architecture](../06-infrastructure/deployment-architecture.md) y [Docker Strategy](../06-infrastructure/docker-strategy.md) (antes: los cuatro servicios en Docker Compose).

## Context

El servidor de producción ya tiene **PostgreSQL** y **nginx** instalados como servicios del sistema, y el owner no quiere duplicarlos en contenedores. El diseño original (Phase 0) asumía un VPS vacío con nginx, PostgreSQL, Mosquitto y la API en Compose.

## Decision

1. **PostgreSQL nativo** (versión ≥ 15, requerida por `NULLS NOT DISTINCT`). `deploy.sh` lo aprovisiona de forma idempotente como el superusuario local (autenticación *peer*, sin contraseña de superusuario): roles `jarvis_migrator`, `jarvis_app`, `jarvis_maintenance`, `jarvis_backup`, base `jarvis`, esquema `jarvis` (propiedad del migrator) y extensión `citext` ([deploy/postgres/provision.sql](../../deploy/postgres/provision.sql)).
2. **nginx nativo**: `deploy.sh` instala un sitio propio (`sites-available/jarvis.conf` o `conf.d/jarvis.conf`) sin `default_server` y con nombres prefijados `jarvis_` (zonas de rate limit, `log_format`, `map`, `upstream`) para convivir con otros sitios. Siempre `nginx -t` antes de recargar; si falla, restaura la configuración anterior.
3. **Docker solo para `jarvis-api` y `mosquitto`**:
   - `jarvis-api` usa `network_mode: host` y escucha **solo en `127.0.0.1`** (8080 API/hub, 8081 métricas). Así alcanza PostgreSQL por `127.0.0.1:5432` sin cambiar `listen_addresses` ni `pg_hba.conf`, y nginx le hace proxy por loopback. `X-Forwarded-*` solo se acepta desde loopback.
   - `mosquitto` en una red bridge: `8883` publicado a Internet (TLS) y `1883` publicado **solo en `127.0.0.1`** para el cloud client.
4. **Certificados**: certbot en modo webroot usando el nginx nativo; un *renewal hook* recarga nginx (`systemctl reload nginx`) y copia el certificado para Mosquitto.
5. **Backups**: `pg_dump` del host con el rol `jarvis_backup` (la versión de `pg_dump` coincide con la del servidor).

## Alternatives

| Alternativa | Por qué no |
|---|---|
| PostgreSQL y nginx en Docker (diseño original) | Duplica servicios ya instalados y gestionados por el owner; dos nginx compitiendo por 80/443. |
| API en red bridge + `host.docker.internal`/`host-gateway` | Obliga a que PostgreSQL escuche en la IP del bridge y a abrir `pg_hba.conf` a la subred de Docker; más cambios en un PostgreSQL compartido. |
| API nativa como servicio systemd | Viable, pero pierde el aislamiento del contenedor (sistema de archivos de solo lectura, `cap_drop: ALL`, `no-new-privileges`) y la imagen reproducible. |
| Mosquitto nativo | Posible; se mantiene en contenedor porque su configuración, entrypoint de Dynamic Security y pruebas de integración ya usan esa imagen exacta. |

## Consequences

- (+) Se reutilizan los servicios existentes; un único `deploy.sh` idempotente cubre primer despliegue y actualizaciones.
- (+) PostgreSQL no se expone: la API lo usa por loopback con usuario/contraseña; el superusuario solo se usa localmente por *peer*.
- (−) Con red del host, el aislamiento de red del contenedor de la API desaparece; se compensa enlazando solo `127.0.0.1` y con el firewall del host (solo 22/80/443/8883).
- (−) `pg_hba.conf` debe permitir `host <db> all 127.0.0.1/32 scram-sha-256` (valor por defecto en Ubuntu/Debian). `deploy.sh` lo verifica y falla con un mensaje claro si no.
- (−) El owner administra actualizaciones de PostgreSQL y nginx con su gestor de paquetes; Jarvis exige PostgreSQL ≥ 15.
- Desarrollo local y pruebas no cambian: siguen usando PostgreSQL y Mosquitto en contenedores.
