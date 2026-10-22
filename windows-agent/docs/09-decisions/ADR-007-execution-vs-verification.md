# ADR-007: Separar ejecución y verificación

## Status

Accepted, 2026-09-19.

## Context

En Windows, que una API devuelva éxito no significa que el usuario obtuvo el resultado (apps que se cierran al abrir, diálogos "¿Guardar?", foreground lock, procesos relanzados). El futuro Agent Core planificará sobre los resultados: un falso éxito contamina todo el razonamiento posterior.

## Decision

> **Jarvis no considera una tarea completada porque la ejecución no produjo una excepción. Una acción se considera completada únicamente cuando el resultado observable satisface la condición de éxito definida por la Tool.**

- Cada tool declara su condición de verificación. Las tools con efectos (R2+) deben tener `verification.mode = required`; solo las de observación pura usan `NOT_APPLICABLE`.
- Fase `Verifying` separada con su propio timeout; el Verifier solo observa.
- `ToolResult` reporta `execution.status` y `verification.status` por separado; `outcome = SUCCEEDED` solo con `PASSED` o `NOT_APPLICABLE`.
- `INCONCLUSIVE` se trata como fallo.

Detalle en [Verification](../03-tools/verification.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Éxito = sin error de API | Falsos éxitos frecuentes |
| Verificación opcional | Se omitiría "por ahora" y nunca volvería |
| Verificación dentro de `execute` | Mezcla efectos y observación; impide reportarlas por separado y probarlas por separado |

## Consequences

- ✅ Resultados fiables para humanos y para el futuro LLM.
- ⚠️ Mayor latencia (sondeo hasta la condición) y algunos falsos negativos con apps lentas (mitigados con timeouts por app y `retryable`).
- ⚠️ Toda tool nueva necesita diseñar su verificación.
