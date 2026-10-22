# Secrets Management

## Inventario

| Secreto | Consumidor | Ubicación en runtime | Generación | Rotación |
|---|---|---|---|---|
| ~~`postgres_superuser_password`~~ | — | — | — | No se usa en producción (ADR-011): el aprovisionamiento corre como `postgres` por autenticación *peer*. `postgres_password` solo existe para `docker-compose.dev.yml` |
| `db_app_password` | jarvis-api (rol `jarvis_app`) | `/run/secrets/db_app_password` | idem | Semestral |
| `db_migrator_password` | paso `migrate` | `/run/secrets/db_migrator_password` | idem | Semestral |
| `db_maintenance_password` | jarvis-api (job retención) | `/run/secrets/db_maintenance_password` | idem | Semestral |
| `db_backup_password` | job de backup (rol `jarvis_backup`) | `/run/secrets/db_backup_password` (contenedor efímero de backup) | idem | Semestral |
| `db_worker_password` | `jarvis-worker` (rol `jarvis_worker`, ADR-016) | `/run/secrets/db_worker_password` | idem | Semestral |
| `jwt_signing_key` | jarvis-api | `/run/secrets/jwt_signing_key` | 32 bytes aleatorios | Semestral, con clave `previous` 1 h |
| `device_secret_pepper` | jarvis-api | `/run/secrets/device_secret_pepper` | 32 bytes aleatorios | **No rotable** sin rotar todas las credenciales de devices (documentado) |
| `mqtt_cloud_password` | jarvis-api (cliente `jarvis-cloud`) | `/run/secrets/mqtt_cloud_password` | idem | Semestral |
| `mosquitto_dynsec_admin_password` | bootstrap de dynsec | Solo en la inicialización | idem | Tras bootstrap, el client admin inicial se elimina o deshabilita |
| `ip_hash_key` | jarvis-api | Derivado diariamente de `jwt_signing_key` con HKDF | — | Diario (automático) |
| `backup_encryption_key` (age) | job de backup | Clave pública en el VPS; **privada fuera del VPS** | `age-keygen` | Anual |
| Certificados TLS | nginx, mosquitto | `/etc/letsencrypt` (nginx, `ro`); copia en `/opt/jarvis/config/mosquitto/certs` (mosquitto) | certbot | Automática (renovación según vida del certificado) |
| Secretos de devices | devices | Fuera del Cloud (NVS, Credential Manager) | API | Por owner vía REST |
| Contraseña del owner | owner | Gestor de contraseñas del owner | owner | A criterio |

## Reglas

1. **Docker secrets en archivo** (`secrets:` de Compose, montados en `/run/secrets/*` con permisos 0400). La API lee rutas vía variables `*_FILE` (p. ej. `Jwt__SigningKeyFile=/run/secrets/jwt_signing_key`).
2. **Nunca** en variables de entorno con el valor (visibles en `docker inspect`, `/proc`), ni en la imagen, ni en `docker-compose.yml`, ni en git.
3. Los archivos de secrets viven en el host en `/opt/jarvis/secrets/` (propiedad `root`, 0700 directorio / 0400 archivos), fuera del directorio del repositorio.
4. `.gitignore` del repo incluye `secrets/`, `*.env`, `*.pem`, `*.key`.
5. Cada servicio monta **solo** los secrets que necesita.
6. La API valida al arrancar que todos los secrets existen y tienen longitud mínima; si no, termina con error explícito (sin imprimir el valor).
7. Secretos en memoria: no se registran; los DTOs que los contienen sobrescriben `ToString()`.
8. Backup de secrets: copia cifrada con `age` a un almacenamiento fuera del VPS, separada del backup de la BD.

## Bootstrap de Mosquitto dynsec

1. Primer arranque: `mosquitto_ctrl dynsec init /mosquitto/data/dynamic-security.json <admin>` con una contraseña temporal.
2. Con el admin temporal: crear roles `jarvis-cloud`, `jarvis-device`; crear client `jarvis-cloud` con `mqtt_cloud_password` y rol `jarvis-cloud`; configurar ACL por defecto a deny.
3. Eliminar el client admin temporal.

Implementación: el paso 1 lo hace `deploy/mosquitto/docker-entrypoint.sh` al crear el volumen; los pasos 2 y 3 los hace `jarvis-api broker-bootstrap --delete-admin`, invocado automáticamente por `deploy/scripts/deploy.sh` la primera vez. Ver [Runbooks](../06-infrastructure/runbooks.md#2-primer-despliegue). Los secretos se generan con `deploy/scripts/generate-secrets.sh`.

## Desarrollo local

- `dotnet user-secrets` o archivos en `./.local-secrets/` (ignorado por git) con los mismos nombres.
- Valores de desarrollo **distintos** de producción; nunca se reutilizan.

### Permisos en el host (Docker Compose)

Compose monta los secretos de archivo con los permisos del host y las imágenes .NET se ejecutan como UID/GID 1654
(no root). `deploy.sh` deja los secretos que leen `jarvis-api` y `jarvis-worker` como `root:1654 0440`; el resto
(`postgres_password`, `db_backup_password`, `mosquitto_dynsec_admin_password`) sigue `root 0400`.
