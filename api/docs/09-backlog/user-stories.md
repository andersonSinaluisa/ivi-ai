# User Stories — índice

Cada HU vive en el archivo de su epic en [stories/](stories/). Para implementar:

```text
Implementa HU-0XX utilizando docs/ como contexto.
```

El agente debe: (1) leer la HU completa, (2) leer los documentos que la HU referencia, (3) respetar los ADRs, (4) no implementar alcance de otras HUs salvo lo mínimo necesario para compilar y probar, (5) actualizar la documentación si algo del diseño resulta imposible, explicando por qué.

## Definition of Done común (aplica a todas las HU)

1. Código compila sin warnings (warnings as errors) y respeta las reglas de dependencia de [Backend Architecture](../01-architecture/backend-architecture.md).
2. Pruebas descritas en la HU implementadas y en verde (`dotnet test`).
3. Sin secretos en código, configuración versionada ni logs.
4. Logs estructurados con `CorrelationId` en los flujos nuevos.
5. Endpoints nuevos: Problem Details con `code` del [Error Model](../03-protocols/error-model.md); autorización probada con un segundo owner.
6. Cambios de estado nuevos: evento registrado en la misma transacción ([Event Model](../02-domain/event-model.md)).
7. Documentación actualizada si la implementación precisó algo no especificado (sin contradecir ADRs; si hay que contradecir, nuevo ADR).
8. Revisión: el diff no introduce conceptos de LLM/proveedor en el Cloud ([ADR-010](../08-decisions/ADR-010-llm-independence.md)).

## Índice

| ID | Título | Epic | Prioridad | Depende de |
|---|---|---|---|---|
| HU-001 | Esqueleto de la solución | 01 | Must | — |
| HU-002 | Configuración tipada y carga de secretos | 01 | Must | 001 |
| HU-003 | Persistencia base PostgreSQL y comando `migrate` | 01 | Must | 002 |
| HU-004 | Logging estructurado y correlation id | 01 | Must | 001 |
| HU-005 | Problem Details y catálogo de errores | 01 | Must | 001 |
| HU-006 | Health checks live/ready | 01 | Must | 003 |
| HU-007 | Entorno local de desarrollo | 01 | Must | 003 |
| HU-008 | Entidad User y comando `bootstrap-owner` | 02 | Must | 003, 038 |
| HU-009 | Login y access token JWT | 02 | Must | 008 |
| HU-010 | Refresh tokens con rotación y logout | 02 | Must | 009 |
| HU-011 | Modelo de dominio Device y estado calculado | 02 | Must | 001 |
| HU-012 | Registrar device | 02 | Must | 011, 018, 043 |
| HU-013 | Listar y consultar devices | 02 | Must | 012 |
| HU-014 | Renombrar, deshabilitar y habilitar device | 02 | Must | 012, 018 |
| HU-015 | Revocar device | 02 | Must | 014, 024 |
| HU-016 | Rotar credencial de device | 02 | Must | 012, 018 |
| HU-017 | Token de device para REST | 03 | Must | 012 |
| HU-018 | Adaptador de administración Dynamic Security | 03 | Must | 020 |
| HU-019 | Reconciliación de credenciales con el broker | 03 | Must | 018 |
| HU-020 | Configuración de Mosquitto, bootstrap dynsec y ACL | 04 | Must | 007 |
| HU-021 | Cliente MQTT del Cloud | 04 | Must | 020, 047 |
| HU-022 | Validación de envelope y router de entrada | 04 | Must | 021 |
| HU-023 | Publicador de mensajes `down` | 04 | Must | 021 |
| HU-024 | Dominio JarvisTask y máquina de estados | 05 | Must | 001 |
| HU-025 | Gestión de task grants | 05 | Must | 012 |
| HU-026 | Crear tarea por REST | 05 | Must | 024, 025, 017 |
| HU-027 | Crear tarea por MQTT | 05 | Must | 026, 022, 023 |
| HU-028 | Dispatcher de tareas | 05 | Must | 026, 023, 035 |
| HU-029 | Procesar actualizaciones de tarea del device | 05 | Must | 028, 022 |
| HU-030 | ACK timeout y redelivery | 05 | Must | 028 |
| HU-031 | Expiración y timeout de ejecución | 05 | Must | 029 |
| HU-032 | Cancelar tarea | 05 | Must | 029 |
| HU-033 | Consultar tareas por REST | 05 | Must | 026 |
| HU-034 | Notificaciones al device origen y `task.get` | 05 | Must | 027, 029 |
| HU-035 | `presence.hello` y reconciliación de tareas abiertas | 06 | Must | 022, 023, 024 |
| HU-036 | Heartbeat, goodbye y LWT | 06 | Must | 035 |
| HU-037 | Presence sweeper | 06 | Must | 036 |
| HU-038 | Infraestructura del event log | 07 | Must | 003 |
| HU-039 | Consultar eventos por REST | 07 | Must | 038, 043 |
| HU-040 | Retención de eventos y tokens | 07 | Should | 038 |
| HU-041 | Hub SignalR con autenticación y groups | 08 | Must | 009 |
| HU-042 | Publicación de notificaciones realtime | 08 | Must | 041, 038 |
| HU-043 | Principal actual, políticas y filtro por owner | 09 | Must | 009 |
| HU-044 | Rate limiting y lockout | 09 | Must | 009, 017 |
| HU-045 | Hardening HTTP de la API | 09 | Must | 004 |
| HU-046 | Verificación de redacción de datos sensibles | 09 | Must | 012, 026 |
| HU-047 | Paquete `Jarvis.Contracts` del protocolo | 10 | Must | 001 |
| HU-048 | Windows Simulator | 10 | Must | 047 |
| HU-049 | ESP32 Simulator | 10 | Must | 047 |
| HU-050 | Escenario E2E del MVP automatizado | 10 | Must | 048, 049, 034, 037 |
| HU-051 | Imagen Docker de la API | 11 | Must | 006 |
| HU-052 | Nginx, TLS y Let's Encrypt | 11 | Must | 051 |
| HU-053 | Docker Compose de producción | 11 | Must | 051, 052, 020 |
| HU-054 | Provisión y hardening del VPS | 11 | Must | — |
| HU-055 | Backups y restauración | 11 | Must | 053 |
| HU-056 | Monitoreo y alertas mínimas | 11 | Should | 053 |
| HU-057 | Aprobación remota de tareas | 05 | Post-MVP | 029, 042 |

## Orden de implementación recomendado

| Paso | HUs | Resultado verificable |
|---|---|---|
| 1 | 001 → 002 → 004 → 005 → 003 → 007 → 006 | API vacía que arranca, migra, loguea y responde health |
| 2 | 047, 024, 011 | Contratos y dominio puro con tests |
| 3 | 038 → 008 → 009 → 043 → 010 | Login del owner con auditoría |
| 4 | 020 → 018 → 012 → 013 → 014 → 016 → 015 → 019 → 017 | Devices registrables con credencial MQTT funcional |
| 5 | 021 → 022 → 023 → 035 → 036 → 037 | Presencia de devices visible por REST |
| 6 | 025 → 026 → 028 → 029 → 030 → 031 → 032 → 033 → 027 → 034 | Ciclo de vida completo de tareas |
| 7 | 039, 041, 042, 044, 045, 046 | Auditoría, realtime y seguridad transversal |
| 8 | 048 → 049 → 050 | Demo E2E automatizada local |
| 9 | 054 → 051 → 052 → 053 → 055 → 056 → 040 | Despliegue en el VPS y demo E2E contra producción |

**Primera HU: HU-001.** Todas las demás dependen de la estructura de proyectos, reglas de dependencia y convenciones de build que define.
