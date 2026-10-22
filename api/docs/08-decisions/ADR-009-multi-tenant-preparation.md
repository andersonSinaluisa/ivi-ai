# ADR-009 — Multi-tenant preparation

- **Status:** Accepted
- **Fecha:** 2026-09-19

## Context

Inicialmente habrá un solo usuario. No se debe hardcodear un usuario global, pero tampoco construir organizaciones, roles e invitaciones que nadie usará aún.

## Decision

- Existe la entidad `User`; **todo recurso tiene `owner_id`** (FK a `users`): devices, tareas, grants, eventos.
- Todo acceso filtra por el `owner_id` del principal: en handlers **y** con global query filter de EF Core (defensa en profundidad).
- Los recursos ajenos responden 404.
- Tokens de users y devices llevan el claim `owner_id`.
- Los grants validan que origen y destino sean del mismo owner.
- SignalR agrupa por `owner:{ownerId}`; el cliente no elige groups.
- No existe registro público: `bootstrap-owner` crea users desde la línea de comandos del servidor.
- **No** se implementan: organizaciones, membresías, roles, invitaciones, cuotas por plan.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Usuario global implícito (sin `owner_id`) | Migración dolorosa después; imposible compartir el Cloud. |
| Modelo completo de tenants/organizaciones/roles | Complejidad sin uso actual. |
| Base de datos o esquema por tenant | Excesivo para el volumen y la operación de un VPS. |
| Row-Level Security de PostgreSQL | Defensa fuerte, pero complica el pooling de conexiones y los background services; se reconsiderará si se abre a terceros. |

## Consequences

- (+) Agregar un segundo owner no requiere cambios de esquema.
- (+) Pruebas con dos owners detectan IDOR desde el MVP.
- (−) Cuando se necesite compartir devices entre personas, habrá que introducir `owner_id` → `tenant_id` + membresías. La migración es mecánica porque todo ya está particionado lógicamente por owner.
