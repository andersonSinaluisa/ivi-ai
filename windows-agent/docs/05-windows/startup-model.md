# Startup Model

## Opciones analizadas

| Criterio | Windows Service | **Scheduled Task (at logon)** | Startup Application (Run key / carpeta Inicio) |
|---|---|---|---|
| Sesión | **Session 0**: sin acceso al escritorio del usuario | Sesión interactiva del usuario | Sesión interactiva |
| Ver/lanzar apps del usuario | ❌ Requiere `WTSQueryUserToken` + `CreateProcessAsUser` (necesita `LocalSystem`) | ✅ | ✅ |
| UIA / ventanas / foco | ❌ | ✅ | ✅ |
| Privilegios | Normalmente `LocalSystem`: viola el [Privilege Model](../04-security/privilege-model.md) | Usuario estándar, `LeastPrivilege` | Usuario estándar |
| Arranca sin login | ✅ | ❌ | ❌ |
| Reinicio automático si cae | ✅ (Service Recovery) | ✅ (`RestartOnFailure`) | ❌ |
| Retraso configurable / condiciones | Limitado | ✅ (delay, sin batería, red disponible) | ❌ |
| Instalación sin admin | ❌ | ✅ (tarea del propio usuario) | ✅ |
| Visibilidad para el usuario | Services.msc | Task Scheduler | Administrador de tareas → Inicio |

## Decisión

**Scheduled Task por usuario, disparada al iniciar sesión** ([ADR-010](../09-decisions/ADR-010-agent-startup-model.md)).

- Trigger `LogonTrigger` para el usuario actual, delay 15 s.
- `RunLevel = LeastPrivilege`, `LogonType = InteractiveToken`.
- `RestartOnFailure`: 3 intentos cada 1 min. Además, el propio agente registra la última caída.
- `MultipleInstancesPolicy = IgnoreNew`; `ExecutionTimeLimit = PT0S` (sin límite).
- `DisallowStartIfOnBatteries = false`, `StopIfGoingOnBatteries = false`.
- Acción: `%LOCALAPPDATA%\Programs\Jarvis\Agent\jarvis-agent.exe run`, sin ventana de consola (subsistema `windows` para el binario del agente o `CREATE_NO_WINDOW`; la CLI se usa en la consola del usuario).

`jarvis-agent install` la crea con la API COM del Task Scheduler (`ITaskService`) sin necesidad de admin; `uninstall` la elimina.

### Por qué no Windows Service

El agente existe para actuar sobre el escritorio del usuario. En Session 0 no ve ventanas, no puede lanzar apps visibles ni usar UIA. Hacerlo funcionar exigiría un servicio `LocalSystem` que genera procesos en la sesión del usuario: un **componente privilegiado permanente** cuya superficie de ataque es mucho mayor que la del agente entero. Solo tendría sentido para capacidades sin UI que deban funcionar antes del login, fuera del alcance actual.

### Riesgos aceptados

| Riesgo | Mitigación |
|---|---|
| Sin login no hay agente | Presencia `OFFLINE` en el Cloud; comportamiento esperado |
| El usuario puede deshabilitar la tarea | Es su PC; `status` lo muestra |
| Tras un crash, hasta 1 min offline | `RestartOnFailure` + recuperación ([Crash Recovery](../06-state/crash-recovery.md)) |

## Ciclo de vida del proceso

1. Instancia única (`Local\Jarvis.Agent.<SID>`).
2. Cargar `config.toml`; validar `policy.toml` (inválida → fail-closed).
3. Abrir SQLite, migrar, `quick_check`; recuperación de arranque.
4. Health `STARTING` → abrir named pipe de la CLI.
5. Leer credencial → Cloud Link (conexión, hello).
6. Registrar notificaciones de sesión y energía.
7. Health `RUNNING`.

Apagado ordenado (`WM_QUERYENDSESSION`, `WM_ENDSESSION`, Ctrl+C, `jarvis-agent stop` por pipe): dejar de aceptar tareas, esperar hasta 5 s a las ejecuciones cancelables, `presence.goodbye`, flush de logs, cerrar BD. Si Windows cierra antes, la recuperación del siguiente arranque cubre lo pendiente.

## Actualizaciones

Implementado en P8 (`jarvis-agent update [--check] [--manifest-file <ruta>] [--apply]`, código en `crates/jarvis-agent/src/update.rs`). Sigue siendo válida la instalación manual con `jarvis-agent install`. Flujo:

```mermaid
sequenceDiagram
    autonumber
    participant A as Agent v1
    participant U as Update source (HTTPS)
    participant S as Stager (jarvis-agent update)
    participant T as Scheduled Task
    A->>U: GET manifest.json (+ manifest.sig)
    A->>A: verificar firma Ed25519 (clave pública embebida)
    A->>A: version > actual (anti-downgrade) y canal correcto
    A->>U: GET jarvis-agent-2.0.0.exe
    A->>A: SHA-256 == manifest y firma Authenticode válida
    A->>S: preparar: copiar a versions\2.0.0\, conservar versions\1.x\
    S->>S: esperar a 0 tareas en Running (o ventana de mantenimiento)
    S->>T: apuntar la tarea a versions\2.0.0\ y reiniciar el agente
    Note over S: v2 debe reportar health RUNNING y conectar en ≤ 120 s
    alt v2 sano
        S->>S: marcar 2.0.0 como buena; borrar versiones antiguas (conservar 1)
    else v2 no arranca / crash loop
        S->>T: rollback a versions\1.x\
        S->>A: health UPDATE_ROLLED_BACK
    end
```

| Aspecto | Decisión |
|---|---|
| Integridad | SHA-256 en manifiesto firmado |
| Autenticidad | Ed25519 con clave **offline** (la clave del Cloud no basta: un Cloud comprometido no debe poder distribuir binarios) + Authenticode |
| Versión | SemVer; anti-downgrade salvo rollback local |
| Migraciones de BD | Solo aditivas entre versiones menores; rollback compatible (la versión anterior ignora columnas nuevas). Backup de `agent.db` antes de migrar |
| Rollback | Automático si la nueva versión no llega a `RUNNING` + conexión en 120 s o cae 3 veces en 10 min |
| Quién lo dispara | Consulta periódica (diaria) o `jarvis-agent update`; el Cloud puede **sugerir** versión, nunca enviar el binario |
