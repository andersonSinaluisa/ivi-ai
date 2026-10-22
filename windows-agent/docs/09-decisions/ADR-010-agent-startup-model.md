# ADR-010: Scheduled Task por usuario al iniciar sesión

## Status

Accepted, 2026-09-19.

## Context

El agente debe arrancar solo y reiniciarse si cae. Sus capacidades (lanzar apps visibles, observar ventanas, UIA) requieren la **sesión interactiva** del usuario. Un Windows Service corre en Session 0, aislado del escritorio.

## Decision

Scheduled Task del propio usuario, `LogonTrigger` con 15 s de retraso, `LeastPrivilege`, `InteractiveToken`, `RestartOnFailure`, `IgnoreNew`, sin límite de tiempo. Creada por `jarvis-agent install` sin admin. Detalle en [Startup Model](../05-windows/startup-model.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Windows Service | Session 0: no ve el escritorio. Para superarlo haría falta un servicio `LocalSystem` que cree procesos en la sesión del usuario: componente privilegiado permanente. Se analizó y **no** se eligió |
| Run key / carpeta Inicio | Sin reinicio automático ni retraso; fácil de romper por limpiadores |
| Servicio + agente de sesión (arquitectura dividida) | Útil solo para funciones previas al login, fuera de alcance; duplica IPC y superficie |

## Consequences

- ✅ Sesión correcta, sin privilegios, instalación por usuario.
- ⚠️ Sin agente antes del login (device `OFFLINE`), aceptado.
- ⚠️ Un agente por usuario de Windows (un `deviceId` por usuario).
