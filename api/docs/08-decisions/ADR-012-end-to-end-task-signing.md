# ADR-012 — Firma end-to-end de tareas y de decisiones de aprobación

- **Status:** Accepted
- **Fecha:** 2026-09-23
- **Completa:** [C10](../00-overview/cloud-compatibility.md) del Windows Agent y [HU-057](../09-backlog/stories/EPIC-05-task-management.md#hu-057--aprobación-remota-de-tareas-post-mvp-phase-3); hace efectivo el campo `signature` que [MQTT §4.3](../03-protocols/mqtt-protocol.md) reservaba en v1.

## Context

El Cloud transporta tareas que un device ejecuta sobre la máquina del owner. El [threat model](../05-security/security-architecture.md) admite un escenario que el MVP no mitiga: **si el VPS es comprometido, el atacante puede inyectar tareas** y el agente las ve exactamente igual que las legítimas, porque `requestedBy` es una identidad *afirmada por el Cloud*.

El Windows Agent ya implementa la mitad receptora (HU-WIN-061 y HU-WIN-062): verifica firmas Ed25519 de tareas contra claves declaradas por principal en su `policy.toml`, y decisiones de aprobación remotas con ventana de frescura y nonce de un solo uso. Hasta ahora era código inerte: el Cloud enviaba `signature: null` y nunca emitía decisiones.

La pregunta que decide este ADR es **quién tiene la clave privada**.

## Decision

**La firma es end-to-end entre el cliente del principal y el agente. El Cloud no firma, no verifica y no guarda claves.**

1. **Quién firma.** El cliente que origina la petición (Owner Console, móvil, CLI, un device con su propia clave). La clave privada nunca sale de él.
2. **Quién verifica.** Solo el device destino, contra las claves públicas de su `policy.toml` (`[[principals]] public_keys` para tareas, `[signatures] approver_keys` para aprobaciones). **El trust anchor es local al agente.**
3. **Qué hace el Cloud.** Persistir la firma y retransmitirla **verbatim**. Valida únicamente la forma: `alg = "Ed25519"`, `keyId` de 1–64 caracteres `[A-Za-z0-9_.:-]`, y `value` base64 que decodifica a exactamente 64 bytes. Una firma con forma válida pero criptográficamente falsa es responsabilidad del agente, que la rechaza con `SIGNATURE_INVALID`.
4. **El Cloud no almacena claves públicas.** No hay endpoint de registro de claves ni tabla de claves. El owner instala la clave pública en el agente (`policy.toml`), que es donde se decide en qué confiar.
5. **`taskId` y `expiresAt` los aporta el cliente cuando firma.** Ambos forman parte de la cadena firmada y el cliente no puede conocerlos si los genera el servidor. En una creación firmada son obligatorios; el Cloud los usa tal cual y valida que `expiresAt` caiga dentro de los límites permitidos y que el `taskId` no esté en uso.
6. **La firma es opcional.** Una tarea sin firma sigue siendo válida: el agente decide por política a partir de qué riesgo la exige (`[signatures] require_above`). Esto mantiene operativos al ESP32 y a cualquier cliente que aún no firme.
7. **Cadenas firmadas** (definidas en [MQTT §11](../03-protocols/mqtt-protocol.md) con vectores de prueba en [`signature-vectors.json`](../03-protocols/signature-vectors.json)):

   ```text
   jarvis.task-signature/1\ntaskId=…\ntype=…\npayloadSha256=…\nrequestedBy=<type>:<id>\nexpiresAt=…
   jarvis.approval-decision/1\ntaskId=…\ndecision=…\npayloadSha256=…\nnonce=…\ndecidedAt=…
   ```

   Se firma una **cadena canónica explícita**, no el JSON recibido, para que el orden de claves o los espacios no cambien el resultado. El payload entra por su `sha256` sobre JSON canónico ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) restringido a los tipos del protocolo). Las marcas de tiempo van en RFC 3339 UTC con milisegundos (`2026-09-23T10:00:00.000Z`).

## Alternatives

| Alternativa | Por qué no |
|---|---|
| **El Cloud firma las tareas** con una clave propia | No mitiga nada: el riesgo que se quiere cubrir es justamente el Cloud comprometido. Firmar con la clave del atacante-en-potencia es teatro de seguridad. |
| El Cloud guarda claves públicas y **verifica** antes de retransmitir | Añade una dependencia criptográfica (.NET 10 no trae Ed25519 en el BCL) para una comprobación que no es frontera de seguridad: el agente vuelve a verificar de todos modos. Peor, un registro de claves en el Cloud invita a creer que revocar ahí protege, cuando quien decide es `policy.toml`. |
| `taskId` y `expiresAt` los sigue generando el Cloud, y el cliente firma **después** en una segunda llamada | Dos fases con una ventana en la que la tarea existe sin firma, y una carrera con el dispatcher. |
| Firmar el JSON del mensaje completo en vez de una cadena canónica | Obliga a que emisor y receptor serialicen idénticamente (orden de claves, escapes, espacios); es la fuente clásica de firmas que no validan. |
| JWS/COSE en lugar de una cadena propia | Formato más rico y más superficie de parseo en el agente (que es el punto sensible), a cambio de nada que necesitemos aquí. |

## Consequences

- **Un Cloud comprometido no puede forjar una tarea firmada.** Puede omitirla, retrasarla, duplicarla o crear tareas *sin* firma; lo último lo corta la política del agente cuando exige firma por encima de cierto riesgo.
- **Perder la clave privada del cliente = perder la capacidad de pedir acciones de riesgo alto**, hasta registrar una clave nueva en el agente. No hay recuperación por el Cloud, y es deliberado.
- **El Cloud no puede explicar por qué una firma falló.** El diagnóstico sale del agente (`SIGNATURE_INVALID` en `task.failed`, con detalle en su log local).
- **Los `taskId` de tareas firmadas los elige el cliente.** Se exige UUID y unicidad global; un id ya usado responde `409 TASK_ID_IN_USE` sin revelar de quién era.
- **La Owner Console (Phase 3) debe gestionar claves**: generarlas, guardarlas en el dispositivo del owner y mostrarle la pública para que la instale en el agente. Es trabajo suyo, no del Cloud.
- Los vectores de prueba fijan la canonicalización para el siguiente cliente que se escriba (navegador, móvil, ESP32) sin tener que leer el código del agente.
