# Threat Model (inicial)

Metodología: STRIDE simplificado sobre las fronteras de [Trust Boundaries](../01-architecture/trust-boundaries.md). Escala: Likelihood y Impact en **Baja / Media / Alta**. El impacto considera el sistema **futuro** (agente real en Windows), no solo los simuladores del MVP, para no diseñar controles que luego sean insuficientes.

## Activos

| Activo | Por qué importa |
|---|---|
| Capacidad de crear tareas para el Windows device | Equivale (en fases futuras) a actuar sobre la PC del owner |
| Credenciales de devices y tokens de users | Dan esa capacidad |
| Event log | Evidencia forense |
| Datos de tareas (payload/result) | Pueden contener información personal |
| Disponibilidad del Cloud | Sin Cloud no hay control remoto |

## Amenazas

### T01 — Robo de credencial de un ESP32
- **Threat:** alguien extrae el secreto (dump de flash, logs de desarrollo, red de provisión) y se conecta como el ESP32.
- **Likelihood:** Media (hardware accesible, flash sin cifrar por defecto).
- **Impact:** Media. El atacante solo puede lo que el ESP32 puede: crear los `(target, taskType)` concedidos por grants (p. ej. `OPEN_CHROME`) y leer notificaciones de sus propias tareas. No puede suscribirse a otros devices ni leer eventos (endpoints de User).
- **Mitigation:** grants mínimos por tipo; ESP32 con *flash encryption* + NVS cifrado (Phase 6); rate limit por device; revocación inmediata (`deleteClient` desconecta); detección: conexión simultánea con el mismo clientId provoca desconexiones alternas visibles como `DEVICE_CONNECTED` repetidos → alerta si > 10/h.
- **Residual Risk:** Bajo-Medio. Hasta la revocación, el atacante puede disparar las tareas concedidas.

### T02 — Compromiso del VPS
- **Threat:** RCE en la API, nginx o Mosquitto, o acceso SSH robado.
- **Likelihood:** Baja-Media.
- **Impact:** Alta. El atacante puede leer la BD (hashes, payloads), emitir tareas a cualquier device, deshabilitar auditoría futura, suplantar al Cloud ante devices.
- **Mitigation:** superficie mínima (3 puertos); SSH solo con llave + fail2ban; actualizaciones automáticas de seguridad; contenedores no-root, `read_only`, `no-new-privileges`, `cap_drop: ALL`; secretos solo en archivos de secrets; **el agente Windows no confía ciegamente en el Cloud** (allowlist local, validación de payload, aprobación local para riesgo alto); firma end-to-end en Phase 3; backups cifrados fuera del VPS.
- **Residual Risk:** Medio en MVP (sin firma). Bajo tras Phase 3 para tareas sensibles.

### T03 — MQTT impersonation
- **Threat:** un cliente se hace pasar por otro device o por el Cloud en el broker.
- **Likelihood:** Baja.
- **Impact:** Alta (si lograra publicar en `down/tasks` de otro device).
- **Mitigation:** `allow_anonymous false`; dynsec con `clientid` fijado = `username` = `deviceId`; ACL `%u`; solo `jarvis-cloud` puede publicar en `down/#`, y su listener (1883) no está expuesto; el Cloud valida `envelope.deviceId == topic` y que el emisor sea el target de la tarea; prueba de integración de ACL obligatoria.
- **Residual Risk:** Bajo.

### T04 — Replay attack
- **Threat:** reenviar un mensaje o request capturado (p. ej. `task.create` o `task.dispatch`).
- **Likelihood:** Baja (TLS impide la captura en red).
- **Impact:** Media.
- **Mitigation:** TLS; `idempotencyKey` obligatoria para devices (un replay devuelve la misma tarea); `ts` ±5 min en `up/requests`; dedup por `taskId` y rechazo por `expiresAt` en el agente; JWT de vida corta.
- **Residual Risk:** Bajo.

### T05 — Task injection
- **Threat:** crear tareas no autorizadas o con payload malicioso (p. ej. `payload.url = "file:///C:/..."` o comandos embebidos).
- **Likelihood:** Media.
- **Impact:** Alta en fases futuras.
- **Mitigation:** el Cloud exige grants y valida formato/tamaño; **no** existe un tipo de tarea genérico de shell; el agente valida el payload contra un esquema estricto por tipo (p. ej. `OPEN_CHROME.url` solo `https://`), aplica allowlist y aprobación; firma en Phase 3.
- **Residual Risk:** Bajo-Medio (depende de la calidad de los validadores del agente).

### T06 — Acceso cruzado entre owners
- **Threat:** un user o device accede a devices/tareas/eventos de otro owner (IDOR).
- **Likelihood:** Media (error clásico de implementación).
- **Impact:** Alta.
- **Mitigation:** toda consulta filtra por `owner_id` del principal (filtro global de EF Core por owner + verificación explícita en handlers); recursos ajenos → 404; grants validan que source y target sean del mismo owner; SignalR sin groups elegidos por el cliente; tests de autorización por endpoint con dos owners ([Acceptance Tests](../10-testing/acceptance-tests.md)).
- **Residual Risk:** Bajo.

### T07 — ESP32 robado
- **Threat:** robo físico del dispositivo con credencial en NVS.
- **Likelihood:** Media.
- **Impact:** Media (igual que T01) y el atacante puede pulsar botones legítimos.
- **Mitigation:** revocación desde la API (efecto inmediato en MQTT); grants mínimos; tareas de riesgo alto nunca concedidas a ESP32 o siempre con aprobación en el agente; flash encryption.
- **Residual Risk:** Bajo tras revocar.

### T08 — Windows Agent malicioso o comprometido
- **Threat:** el agente (o malware en la PC) publica resultados falsos, inunda el Cloud o intenta actuar sobre otros devices.
- **Likelihood:** Baja-Media.
- **Impact:** Media: puede mentir sobre sus propias tareas y generar ruido; **no** puede crear tareas hacia otros devices sin grants ni leer datos de otros devices.
- **Mitigation:** ACL por device; el Cloud solo acepta `up/tasks` del target; límites de tamaño y rate; grants inexistentes por defecto para el agente; resultados tratados como datos no confiables (nunca se interpretan ni renderizan como HTML).
- **Residual Risk:** Bajo para el Cloud. La PC comprometida está fuera del alcance del Cloud.

### T09 — Man-in-the-middle
- **Threat:** interceptar o modificar tráfico device ↔ Cloud o UI ↔ Cloud.
- **Likelihood:** Baja.
- **Impact:** Alta.
- **Mitigation:** TLS 1.2+ en 443 y 8883; los devices **validan** certificado y hostname (prohibido `skip_cert_common_name_check` o equivalente en producción); ISRG Root X1/X2 embebidas; HSTS.
- **Residual Risk:** Bajo. Riesgo operativo: cambio de cadena de Let's Encrypt → mantener ambas raíces.

### T10 — Brute force
- **Threat:** adivinar contraseña del owner o secretos de device.
- **Likelihood:** Alta (intentos automáticos constantes en Internet).
- **Impact:** Alta si tiene éxito contra el owner.
- **Mitigation:** secretos de device de 256 bits (inviables de adivinar); contraseña ≥ 12 caracteres + lista de comunes; lockout; rate limit nginx y API; mensajes de error indistinguibles; fail2ban opcional sobre logs de nginx y Mosquitto.
- **Residual Risk:** Bajo.

### T11 — Denial of service
- **Threat:** saturar nginx, Mosquitto o la API; agotar disco con eventos o logs.
- **Likelihood:** Media.
- **Impact:** Media (pérdida de control remoto; sin pérdida de datos).
- **Mitigation:** rate limits por capa; `max_connections`, `max_packet_size`, `max_queued_messages` en Mosquitto; límites de memoria/CPU por contenedor; rotación de logs Docker; límites de tamaño de payload/result/eventos; retención; alertas de disco > 80 %. DDoS volumétrico queda fuera de alcance (opción futura: proxy anti-DDoS delante de 443; 8883 no puede ir detrás de un CDN HTTP).
- **Residual Risk:** Medio frente a DDoS volumétrico; Bajo frente a abuso de aplicación.

### T12 — Filtración de secretos en logs
- **Threat:** secretos o tokens terminan en logs, traces, excepciones o backups sin cifrar.
- **Likelihood:** Media.
- **Impact:** Alta.
- **Mitigation:** política de redacción ([Observability](../06-infrastructure/observability.md)); DTOs de logging explícitos (no se loguean objetos de request completos); nginx con formato de log sin query string ni `Authorization`; test automatizado que registra una creación de device y verifica que el secreto no aparece en la salida de logs; backups cifrados.
- **Residual Risk:** Bajo.

### T13 — Compromiso de la cuenta del owner
- **Threat:** phishing o reutilización de contraseña.
- **Likelihood:** Media.
- **Impact:** Alta (control total de devices y grants).
- **Mitigation:** lockout, eventos `USER_LOGIN_*` auditables, refresh token reuse detection; MFA antes de Phase 3; aprobación local en el agente para tareas sensibles incluso si vienen del owner.
- **Residual Risk:** Medio hasta MFA.

## Resumen

| ID | Amenaza | L | I | Residual |
|---|---|---|---|---|
| T01 | Robo de credencial ESP32 | M | M | Bajo-Medio |
| T02 | Compromiso del VPS | B-M | A | Medio (MVP) |
| T03 | MQTT impersonation | B | A | Bajo |
| T04 | Replay | B | M | Bajo |
| T05 | Task injection | M | A | Bajo-Medio |
| T06 | Acceso cruzado entre owners | M | A | Bajo |
| T07 | ESP32 robado | M | M | Bajo |
| T08 | Windows Agent malicioso | B-M | M | Bajo |
| T09 | MITM | B | A | Bajo |
| T10 | Brute force | A | A | Bajo |
| T11 | DoS | M | M | Medio |
| T12 | Secretos en logs | M | A | Bajo |
| T13 | Cuenta del owner comprometida | M | A | Medio |
