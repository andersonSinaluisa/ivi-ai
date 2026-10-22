# ADR-002: Estrategia jerárquica de integración con Windows

## Status

Accepted, 2026-09-19.

## Context

Hay muchas formas de "hacer algo" en Windows: APIs nativas, Win32, UI Automation, integraciones por app, visión, input sintético. Difieren enormemente en fiabilidad, verificabilidad, privacidad e interferencia con el usuario.

## Decision

Cada tool usa el nivel más alto disponible de: **1 Native API → 2 Win32 → 3 UI Automation → 4 Application-specific → 5 Computer Vision → 6 Raw mouse/keyboard**. Bajar de nivel se justifica en el diseño de la tool. Input crudo es el último recurso, siempre R3+. Detalle en [Windows Integration](../05-windows/windows-integration.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Visión + input como mecanismo universal ("computer use") | Genérico pero frágil, no verificable semánticamente, invasivo para la privacidad e interfiere con el usuario |
| Solo Win32 | Insuficiente para interactuar con el contenido de apps (P9+) |
| Scripts AutoHotkey/PowerShell | Runtime externo, ejecución de scripts, difícil de acotar por política |

## Consequences

- ✅ Acciones verificables y autorizables por semántica.
- ✅ El MVP solo necesita niveles 1–2.
- ⚠️ Más trabajo por capacidad (una integración adecuada por caso).
- ⚠️ Algunas apps no serán automatizables hasta P12, y es aceptable.
