# ADR-004: Autoridad de permisos local

## Status

Accepted, 2026-09-19.

## Context

El Cloud tiene su propia autorización (ownership, device types, supportedTaskTypes). Pero el Cloud es un VPS expuesto a Internet; si se compromete, cualquier autorización que viva solo allí se compromete con él. El agente controla la PC personal del usuario.

## Decision

La **política local** (`policy.toml`, editable solo por el usuario en la PC) es la autoridad final. El Cloud solicita; el agente decide `APPROVED`, `REQUIRES_APPROVAL` o `DENIED` según principal, capabilities y riesgo efectivo. No existe mensaje de protocolo que modifique la política. R4 remoto está denegado por código. Política por defecto: ningún principal remoto. Detalle en [Permission Engine](../04-security/permission-engine.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Confiar en la autorización del Cloud | Un único punto de compromiso da control total de la PC |
| Política gestionada desde el Cloud (sincronizada) | Cómodo, pero reintroduce el problema: un Cloud comprometido cambiaría la política. Posible futuro: políticas **firmadas** por una clave del owner que no esté en el Cloud |
| Aprobación humana para todo | Seguro pero inutilizable (el botón del ESP32 perdería sentido) |

## Consequences

- ✅ Compromiso del Cloud acotado a lo concedido localmente (R0–R2 por defecto).
- ⚠️ Configuración duplicada: el owner configura permisos en la PC además de en el Cloud; `provision` ofrece una política inicial razonable.
- ⚠️ Hasta que exista firma (P8), `requestedBy` es identidad afirmada.
