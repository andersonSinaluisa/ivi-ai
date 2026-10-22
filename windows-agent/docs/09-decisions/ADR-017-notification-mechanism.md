# ADR-017: Notificaciones por globo del área de notificación, sin AUMID

## Status

Accepted, 2026-09-19. Modifica el mecanismo y la verificación de `show_notification` descritos en [Tool System](../03-tools/tool-system.md#catálogo) y [HU-WIN-060](../10-backlog/stories/EPIC-13-post-mvp-tools.md#hu-win-060--tool-show_notification).

## Context

**Decisión previa**: la tool `show_notification` mostraría un *toast* de Windows mediante `ToastNotifier`, y la verificación sería "toast entregado (la API confirma la entrega; estado `Shown` cuando el evento esté disponible, si no `INCONCLUSIVE`)". La historia añadía que una app sin paquete necesita un **AUMID registrado con un acceso directo en el menú Inicio**, creado por `install`.

**Problema encontrado al implementar**:

1. `ToastNotificationManager.CreateToastNotifier(aumid)` exige que el AUMID esté registrado y que el acceso directo tenga la propiedad `System.AppUserModel.ID`. Eso obliga a `install` a escribir en el menú Inicio del usuario y a que el binario no se mueva; si el acceso directo falta o cambia, la tool deja de funcionar con un error que el usuario no puede interpretar. El agente ya se instala sin tocar el menú Inicio ([ADR-010](ADR-010-agent-startup-model.md): solo una Scheduled Task).
2. El agente **ya tiene** un icono del área de notificación con su ventana *message-only*: es el que avisa de aprobaciones pendientes ([HU-WIN-027](../10-backlog/stories/EPIC-06-permission-engine.md#hu-win-027--aprobaciones-locales), [WT-15](../04-security/threat-model.md#wt-15-suplantación-de-la-ui-de-aprobación)). Reutilizarlo no añade superficie nueva.
3. La verificación planteada no era observable: con globos no hay evento `Shown`, y con `ToastNotifier` el estado `Failed`/`Dismissed` llega de forma asíncrona y no siempre. Una verificación que solo puede devolver `PASSED` porque mira lo mismo que ya miró la ejecución no verifica nada; declararla `Required` y devolver siempre `INCONCLUSIVE` haría fallar todas las tareas.

## Decision

1. `show_notification` usa **`Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)`** sobre el icono que el agente ya registra: un globo con título y cuerpo, **sin botones ni acciones** (se mantiene WT-15).
2. El texto es **plano**: se eliminan caracteres de control y se recorta a 64/256 caracteres. Al no construirse XML de toast, no existe la vía de inyección de marcado que motivaba el "se escapa" de la historia.
3. La entrega la confirma **la propia llamada**: si el shell rechaza el globo (sin área de notificación, notificaciones desactivadas, modo concentración) la ejecución termina en `EXECUTION_FAILED` con el motivo. Por eso la tool declara `verification = NOT_APPLICABLE` en vez de una verificación separada: el único observable disponible ya está en la ejecución, y el contrato permite `NOT_APPLICABLE` para una tool R1 ([Tool Contract](../03-tools/tool-contract.md#verificación)).
4. Si en el futuro el agente se distribuye empaquetado (MSIX) o `install` pasa a crear el acceso directo con AUMID, se revisa esta ADR para volver a `ToastNotifier` con verificación por evento.

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| `ToastNotifier` con AUMID registrado por `install` | Obliga a escribir en el menú Inicio y a un binario en ruta fija; rompe si el usuario mueve o borra el acceso directo |
| Empaquetar el agente como MSIX solo para notificar | Cambio de modelo de distribución desproporcionado para una tool R1 |
| Declarar la verificación `Required` y devolver siempre `PASSED` | Sería una verificación falsa: mira exactamente lo mismo que la ejecución |
| `MessageBox` | Ventana modal que roba el foco: peor para el usuario y podría usarse para molestar |

## Consequences

- ✅ Sin dependencias de instalación: la notificación funciona con el agente tal cual se instala hoy.
- ✅ El resultado es honesto: `delivered: true` solo cuando el shell aceptó mostrarlo; si no, la tarea falla con el motivo.
- ⚠️ El aspecto es el de un globo clásico, no el de un toast moderno del centro de notificaciones, y Windows puede colapsarlo si llegan varios seguidos.
- ⚠️ Sin AUMID, la notificación no queda en el historial del centro de notificaciones.
