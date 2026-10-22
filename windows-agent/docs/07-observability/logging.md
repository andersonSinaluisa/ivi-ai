# Logging

## Formato y destino

| Aspecto | Decisión |
|---|---|
| Librería | `tracing` + `tracing-subscriber` (JSON) + `tracing-appender` (rotación) |
| Destino | `%LOCALAPPDATA%\Jarvis\Agent\logs\agent-YYYY-MM-DD.jsonl` |
| Rotación | Diaria y por tamaño (`max_file_mb`, default 20); retención `retention_days` (default 14) |
| Formato | JSON por línea, UTC, `ts`, `level`, `target`, `msg`, campos del span |
| Consola | Solo con `jarvis-agent run --console` (desarrollo), formato legible |
| Windows Event Log | No en MVP: registrar una fuente de eventos requiere admin, lo que contradice la instalación por usuario. Los eventos críticos (`AUTH_REJECTED`, `STATE_RESET`, `POLICY_INVALID`, `SAFE_MODE`) se ven en `status` y opcionalmente como toast |
| Envío al Cloud | **No** en MVP. Los logs se quedan en la PC; el owner los comparte con `diagnostics` |

## Contexto obligatorio

Cada evento dentro del procesamiento de una tarea hereda del span:

| Campo | Ejemplo |
|---|---|
| `taskId` | `0192a6f4-9000-…` |
| `executionId` | `0192a6f4-9250-…` (desde `Running`) |
| `correlationId` | del envelope |
| `tool` | `open_app` |
| `durationMs` | al cerrar el span |
| `result` | `SUCCEEDED` / `FAILED:<code>` / `CANCELLED` |
| `principal` | `cloud:device:…0001` (id, no nombre) |
| `risk`, `decision` | `R2`, `APPROVED` |

## Nunca loggear

| Dato | Cómo se garantiza |
|---|---|
| Passwords, device secret, tokens | Tipos `Secret<T>` cuyo `Debug`/`Display` imprime `***`; filtro por nombre de campo (`password`, `secret`, `token`, `authorization`, `signature.value`) |
| Contenido de archivos (`read_file`) | El output de tools con `sensitivity` ≠ `none` nunca se pasa al logger; solo `bytes`, `sha256` |
| Clipboard | Ninguna tool lo lee en v1; cualquier futura queda marcada `input_and_output` |
| Salida de comandos | Solo `exitCode`, `stdoutBytes`, `stderrBytes` |
| Títulos de ventana | Solo `titleLength`, `titleRedacted` |
| Payload completo de tareas | Solo `payloadHash`, `payloadBytes` y campos no sensibles declarados por la tool (`appId`) |
| Rutas completas del perfil | `%USERPROFILE%` se sustituye por `~` en mensajes |
| Stack traces | Solo en logs locales `DEBUG` ante pánico; **nunca** en mensajes al Cloud |

Esto se verifica con tests: se ejecutan tools con valores **canario** (`CANARY-7f3a…`) en secretos, archivos y salidas, y se comprueba que el canario no aparece en ningún archivo de log ([Security Tests](../11-testing/security-tests.md)).

## Niveles

| Nivel | Uso | Ejemplo |
|---|---|---|
| `TRACE` | Detalle interno, solo desarrollo; nunca activo en builds release salvo `--trace` explícito temporal | Cada sondeo de verificación |
| `DEBUG` | Diagnóstico puntual | Decisión de política con la regla aplicada; reintentos de publicación |
| `INFO` | Ciclo de vida normal (default) | Arranque, conexión, tarea recibida/iniciada/terminada, aprobación decidida |
| `WARN` | Situaciones anómalas recuperables | Mensaje inválido descartado, `AUTH_REJECTED` (1/h), `CLOCK_SKEW`, payload distinto con mismo `taskId` |
| `ERROR` | Fallos que requieren atención | BD corrupta, política inválida, pánico capturado |

Límite de volumen: un mismo mensaje `WARN` repetido se agrega (`repeated: N`) cada 60 s para que un flood MQTT no llene el disco.

## Auditoría

Además de los logs, el State Store es el **registro de auditoría**: cada tarea con principal, riesgo, decisión, regla, aprobación (quién/cuándo), ejecución y verificación. Cambios de `policy.toml` se registran con hash anterior/nuevo en `meta`/log `INFO`.
