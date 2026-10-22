# Diagnostics

## `jarvis-agent diagnostics`

Genera `jarvis-diagnostics-<timestamp>.zip` en el escritorio del usuario para compartir al pedir ayuda. **Nunca** se sube automáticamente.

| Incluye | Redacción |
|---|---|
| Health snapshot | Sin cambios (no contiene secretos) |
| `config.toml`, `policy.toml` | Tal cual (no contienen secretos por diseño; se revalida que no haya claves sospechosas) |
| Últimos 3 días de logs | Segunda pasada de redacción: emails, rutas del perfil, patrones de tokens (`eyJ…`, hex/base64 largos) |
| Resumen de las últimas 200 tareas | `taskId`, tool, estado, riesgo, decisión, códigos de error, duraciones, `verification.checks` sin `observed` sensible. **Sin** payloads ni outputs |
| Versión, build, protocolo | — |
| Info del sistema | Versión mayor de Windows, arquitectura, DPI, número de monitores. **Sin** hostname, usuario, IP ni lista de software |
| Esquema y `PRAGMA integrity_check` de la BD | Solo resultado, no datos |

| Opción | Uso |
|---|---|
| `--no-logs` | Excluye logs |
| `--days N` | Días de logs |
| `--recover-outbox` | Intenta extraer el outbox de un `agent.db.corrupt-*` ([Crash Recovery](../06-state/crash-recovery.md)) |
| `--print` | Muestra el contenido sin generar zip, para revisarlo antes |

## Diagnóstico de tareas

`jarvis-agent tasks show <taskId>` (post-MVP en la CLI; los datos existen desde P3): línea de tiempo local de una tarea (recepción, decisión y regla, aprobación, locks esperados, ejecución, cada check de verificación, mensajes publicados y acks). Es la herramienta principal para responder "¿por qué falló?" sin leer logs.

## Métricas locales

Contadores en memoria expuestos en `status --verbose` (no hay endpoint HTTP: el agente no abre puertos):

| Métrica | Etiquetas |
|---|---|
| `tasks_received_total` | `tool` |
| `tasks_rejected_total` | `code` |
| `tasks_terminal_total` | `tool`, `outcome` |
| `verification_failed_total` | `tool`, `method` |
| `tool_duration_ms` (p50/p95) | `tool` |
| `mqtt_reconnects_total` | — |
| `outbox_pending` | — |
| `lock_wait_ms` (p95) | `lock` |
