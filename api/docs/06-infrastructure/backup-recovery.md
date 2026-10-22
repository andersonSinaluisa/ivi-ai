# Backup & Recovery

> **Producción (ADR-011):** PostgreSQL y nginx son **nativos** en el host; Docker solo ejecuta `jarvis-api` (red del host, escucha en 127.0.0.1) y `mosquitto`. Ver [ADR-011](../08-decisions/ADR-011-native-postgres-nginx.md) y [Runbooks](runbooks.md). Las secciones que describen nginx/PostgreSQL en contenedores quedan como referencia del diseño original y siguen aplicando a desarrollo/pruebas.

## Objetivos

| Métrica | Objetivo MVP |
|---|---|
| RPO (pérdida máxima de datos) | 24 h |
| RTO (tiempo para restaurar servicio) | 2 h |

Justificación: el sistema es personal; perder un día de historial de tareas es tolerable, y las tareas en curso se reintentan desde los devices (idempotency keys). Si el RPO debe bajar, se agregará WAL archiving (p. ej. `pgBackRest` o `wal-g`) a almacenamiento externo.

## Qué se respalda

| Elemento | Método | Frecuencia | Retención |
|---|---|---|---|
| PostgreSQL | `pg_dump -Fc` con rol `jarvis_backup` desde un contenedor efímero en la red `backend` | Diario 03:00 UTC | 7 diarios, 4 semanales, 6 mensuales |
| Integridad | `SHA256SUMS` de los `.age`; el job `backup.verify` de `jarvis-worker` comprueba antigüedad, cabecera `age` y suma (solo lectura) | Diario | — |
| `dynamic-security.json` de Mosquitto | Copia del archivo | Diario (mismo job) | Igual |
| Configuración (`/opt/jarvis/config`, compose) | Versionada en git (sin secretos) | En cada cambio | Git |
| Secrets (`/opt/jarvis/secrets`) | Tar cifrado con `age` | Al cambiar | Últimas 3 versiones |
| Certificados | No (se reemiten) | — | — |
| Logs | No | — | — |

## Almacenamiento

1. El job genera los archivos en `/opt/jarvis/backups/` (local, para restore rápido).
2. Los cifra con `age` usando la **clave pública**; la privada está fuera del VPS (gestor de contraseñas del owner + copia offline).
3. Los sube a un almacenamiento externo compatible S3 u otro host (proveedor: decisión pendiente) con credenciales de **solo escritura** (sin permiso de borrado; retención por lifecycle del bucket). Así un atacante con el VPS no puede borrar los backups remotos.

## Restauración (runbook)

1. Provisionar VPS nuevo según [Deployment Architecture](deployment-architecture.md); restaurar `/opt/jarvis/secrets` desde el backup cifrado.
2. Levantar solo `postgres`; crear roles (script de init).
3. `pg_restore --clean --if-exists -d jarvis backup.dump`.
4. Restaurar `dynamic-security.json` en el volumen de Mosquitto antes de iniciarlo.
5. Levantar el resto; emitir certificados; apuntar DNS.
6. Verificar: `/health/ready`, login, listado de devices, conexión de un simulador.
7. Tareas no terminales al momento del backup: los sweepers las expiran o redespachan; los devices deduplican por `taskId`.

Si no hay backup de `dynamic-security.json`: rotar credenciales de todos los devices ([Persistence Strategy](../04-data/persistence-strategy.md#reconciliación-de-dynsec-sin-secretos-en-claro)).

## Pruebas

- Restauración de prueba en un entorno local **antes** de declarar el MVP completo ([MVP Definition](../09-backlog/mvp-definition.md)).
- Después, trimestral.
