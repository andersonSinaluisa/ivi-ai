# Alcance

## Responsabilidades: Cloud vs Windows Agent

| Tema | Jarvis Cloud | jarvis-windows-agent |
|---|---|---|
| Identidad de devices y users | ✅ emite y revoca credenciales | Guarda su credencial (Credential Manager) y la usa |
| Autorización de **quién puede pedir** | ✅ grants `(origen, destino, TaskType)` | Segunda autorización local por identidad remota |
| Autorización de **qué se ejecuta** | ❌ | ✅ Permission Engine + riesgo + aprobación |
| Transporte, entrega at-least-once, reintentos | ✅ | Consumidor idempotente, ACK tras persistir |
| Estado global de tareas y auditoría | ✅ PostgreSQL, event log | Historial local mínimo (SQLite) para idempotencia y recuperación |
| Presencia | ✅ calcula Online/Stale/Offline | Publica hello/heartbeat/LWT; conoce el estado de sesión de Windows |
| Acceso a Windows | ❌ nunca | ✅ exclusivo |
| Verificación de resultados | ❌ (confía en el reporte) | ✅ Verification Engine |
| LLM / razonamiento | ❌ | ❌ (futuro `jarvis-agent-core`) |

El agente **no depende** de cómo el Cloud persiste datos: solo del [contrato MQTT v1](../02-cloud-integration/jarvis-cloud-contract.md).

## Dentro del alcance (proyecto completo)

- Proceso residente en la sesión interactiva del usuario, arranque automático al iniciar sesión.
- Conexión MQTT/TLS al Cloud, presencia, reconexión, outbox persistente.
- Provisioning y almacenamiento seguro de la credencial.
- Sistema de tools: contrato, registro, ejecución con timeouts y cancelación, verificación.
- Permission Engine, modelo de riesgo, aprobaciones (estado y protocolo).
- Estado local en SQLite: idempotencia, outbox, aprobaciones, recuperación ante caídas.
- Tools de procesos y ventana activa (MVP); filesystem de lectura y comandos con nombre (post-MVP).
- Observabilidad local, health, diagnóstico.
- Empaquetado, instalación por usuario, estrategia de actualización.

## Fuera del alcance (explícitamente excluido)

Claude, OpenAI o cualquier LLM · razonamiento o planificación autónoma · voz (Whisper, TTS) · firmware ESP32 · mobile · memoria / Obsidian · visión por computadora · automatización con mouse/teclado crudo · UI de aprobaciones · navegador (se diseña el boundary, no se implementa).

## Supuestos

| # | Supuesto |
|---|---|
| S1 | Windows 10 22H2 o Windows 11, x64. |
| S2 | Un usuario principal por PC; el agente corre en **su** sesión. |
| S3 | Jarvis Cloud Phase 1 desplegado con el protocolo MQTT v1 ([api/docs](../../../api/docs/03-protocols/mqtt-protocol.md)). |
| S4 | El device se registró en el Cloud como tipo `Windows` y el owner tiene su secreto una vez. |
| S5 | Reloj de Windows sincronizado (NTP); el agente además estima el desfase con `serverTime` del Cloud. |
