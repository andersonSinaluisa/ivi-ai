# Glosario

Términos usados en toda la documentación. Si un documento usa un término con otro significado, este glosario prevalece y el documento debe corregirse.

| Término | Definición |
|---|---|
| **Owner** | Usuario dueño de dispositivos, tareas y eventos. Todo recurso tiene `owner_id`. Entidad `User`. |
| **User** | Persona que se autentica con email + contraseña. En el MVP todo User es Owner de sus recursos. |
| **Device** | Cualquier participante no humano con identidad propia: PC Windows, ESP32, app mobile, cliente web. Tiene un `DeviceType`. |
| **DeviceType** | `Windows`, `ESP32`, `Mobile`, `Web`, `Unknown`. Descriptivo; no otorga permisos. |
| **Capability** | Etiqueta string reportada por el device (`display`, `speaker`, ...) que describe lo que el hardware/software ofrece. Descriptiva; no otorga permisos. |
| **Supported task types** | Lista de `TaskType` que un device declara poder ejecutar (reportada en `presence.hello`). |
| **DeviceCredential** | Secreto de 256 bits asociado a un device. Se muestra una vez; se persiste solo su hash. |
| **Device token** | JWT de vida corta (10 min) obtenido por un device con su credencial para llamar la REST API. |
| **Principal** | Quien hace una petición: `User` o `Device`. |
| **Admin state** | Estado administrativo de un device: `Active`, `Disabled`, `Revoked`. |
| **Presence state** | Estado de conectividad persistido: `Online`, `Stale`, `Offline`. |
| **Device status** | Estado **calculado** expuesto por la API: `Disabled`, `Offline`, `Stale`, `Busy`, `Online` (en ese orden de precedencia). Ver [Device Model](../02-domain/device-model.md). |
| **Task** | Unidad de trabajo estructurada que un principal pide a un device destino. En código la clase se llama `JarvisTask` para evitar colisión con `System.Threading.Tasks.Task`. |
| **TaskType** | String `UPPER_SNAKE_CASE` opaco para el Cloud (p. ej. `OPEN_CHROME`). El Cloud no interpreta su semántica. |
| **Task status** | `Pending`, `Dispatched`, `Delivered`, `Running`, `WaitingForApproval`, `Completed`, `Failed`, `Cancelled`, `Expired`. Ver [Task State Machine](../02-domain/task-state-machine.md). |
| **Terminal status** | `Completed`, `Failed`, `Cancelled`, `Expired`. No admite transiciones. |
| **Source device** | Device que originó la tarea (nulo si la creó un User). |
| **Target device** | Device que debe ejecutar la tarea. |
| **Task grant** | Permiso explícito del owner: *el device S puede crear tareas de tipo T dirigidas al device D*. Tabla `device_task_grants`. |
| **Dispatch** | Publicar la tarea al topic MQTT del target. Pasa la tarea a `Dispatched`. |
| **ACK (task.ack)** | Mensaje del target confirmando que **persistió** la tarea localmente. Pasa la tarea a `Delivered`. |
| **Settled** | Una tarea está *settled* en el device cuando éste recibió `task.update.ack` del Cloud para su estado terminal. |
| **Open task ids** | Ids de tareas que un device conoce y aún no están settled. Se reportan en `presence.hello`. |
| **Envelope** | Estructura JSON común de todo mensaje MQTT (`v`, `id`, `type`, `ts`, `deviceId`, `correlationId`, `data`). |
| **Correlation id** | UUID que agrupa todo lo causado por una misma intención (petición REST, mensaje MQTT, tarea, eventos, logs). |
| **Idempotency key** | Clave provista por el cliente para que reintentos de creación no creen tareas duplicadas. |
| **Event** | Registro append-only del event log operacional (`events`). Nombres `UPPER_SNAKE_CASE`. No es Event Sourcing. |
| **Realtime notification** | Mensaje SignalR derivado de un Event, nombres `lower.dot.case`. No es fuente de verdad. |
| **Session id** | Identificador aleatorio que el device genera en cada conexión MQTT; distingue conexiones para ignorar LWT viejos. |
| **LWT** | Last Will and Testament de MQTT: mensaje que el broker publica si el device se desconecta sin `DISCONNECT` limpio. |
| **Dynsec** | Plugin *Dynamic Security* de Mosquitto: usuarios, roles y ACL administrados en caliente vía topic `$CONTROL`. |
| **Cloud client** | Conexión MQTT de la propia API (`jarvis-cloud`), la única autorizada a publicar en topics `down`. |
| **Simulator** | Aplicación de consola que implementa el protocolo real de un device (Windows o ESP32) sin hardware ni LLM. |
| **Reasoning provider** | Motor que razona en el Windows Agent (Claude CLI inicialmente). Desconocido para el Cloud. |
| **Permission Engine** | Componente futuro del Windows Agent que decide si una tarea puede ejecutarse, con o sin aprobación local. |
