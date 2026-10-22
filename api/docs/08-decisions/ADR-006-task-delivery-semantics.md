# ADR-006 — Task delivery semantics

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Las conexiones de devices se caen, el broker puede reiniciar y la API puede caer entre commit y publicación. Una reconexión no debe provocar una segunda ejecución de una acción peligrosa. "Exactly-once" end-to-end no es alcanzable en sistemas distribuidos con fallos de red; se puede lograr **efecto exactly-once** combinando entrega at-least-once con consumidores idempotentes.

## Decision

1. **At-least-once** del Cloud al device: la tarea persiste en PostgreSQL como `Pending`; se publica con QoS 1; pasa a `Dispatched`; solo `task.ack` del device (tras persistir localmente) la hace `Delivered`. Sin ACK en 30 s → vuelve a `Pending` y se redespacha, hasta `max_attempts = 5` → `Failed DELIVERY_FAILED`.
2. **Consumidor idempotente obligatorio en el device**: `taskId` es la clave de deduplicación; registro durable ≥ 8 días; un `taskId` conocido nunca se re-ejecuta, se responde con el último estado.
3. **Orden de persistencia en el device**: inbox → ack → "iniciando" → ejecutar → outbox → publicar. Si el agente reinicia con una tarea "iniciada" sin resultado, reporta `Failed` (no reejecuta).
4. **At-least-once del device al Cloud** para estados terminales: outbox del device reenvía hasta `task.update.ack`.
5. **Cloud idempotente**: regla de avance monótono en la máquina de estados; updates condicionales.
6. **Idempotencia de creación**: `idempotencyKey` (obligatoria para devices) única por solicitante; misma key + mismo contenido → misma tarea.
7. **Expiración**: `expiresAt` limita la entrega (default 10 min, máx. 7 días); el device rechaza tareas expiradas; el Cloud marca `Expired` solo tareas no entregadas; tareas entregadas que exceden duración → `Failed EXECUTION_TIMEOUT`.
8. **Reconciliación** en cada `presence.hello` mediante `openTaskIds`.
9. **Sin dead-letter queue separada**: las tareas imposibles de entregar terminan `Failed` con `DELIVERY_FAILED`/`EXECUTION_TIMEOUT`/`DEVICE_LOST`, quedan consultables por REST y notificadas por evento. Eso cumple la función de DLQ sin otra infraestructura.
10. **El Cloud nunca reintenta una tarea `Failed`**; reintentar es crear otra tarea.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| At-most-once (QoS 0, sin reintentos) | Pierde tareas en cualquier corte. |
| Confiar en QoS 2 como exactly-once | QoS 2 es exactly-once **por salto** cliente↔broker, no end-to-end; no cubre caídas de la aplicación ni reconexiones con clean start. |
| Deduplicación solo en el Cloud | El Cloud no puede saber si el device ejecutó antes de caerse; la deduplicación debe estar donde ocurre el efecto. |
| Redelivery sin límite | Tareas zombis y tráfico infinito ante firmware defectuoso. |

## Consequences

- (+) Ninguna tarea se pierde por fallos de transporte; ninguna se ejecuta dos veces si el device cumple su contrato.
- (+) Estados explicables y auditables.
- (−) Obligaciones no triviales para cada device ejecutor (inbox/outbox persistentes). El ESP32 como ejecutor necesita un ring buffer en NVS.
- (−) Una tarea puede quedar ejecutada en el device pero `Expired` en el Cloud si el ack se perdió y el device no reconecta antes del vencimiento + gracia (ventana pequeña; el device igualmente reporta el resultado y el Cloud lo registra como `late_terminal_update`). Riesgo aceptado y observable.
