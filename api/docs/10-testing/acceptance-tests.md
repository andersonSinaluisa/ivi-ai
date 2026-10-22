# Acceptance Tests

Verificaciones finales para declarar el MVP ([MVP Definition](../09-backlog/mvp-definition.md)). Se ejecutan contra el VPS de producción salvo indicación. Marcar cada fila con fecha y resultado al cerrar Phase 1.

| ID | Verificación | Cómo | Esperado | Resultado |
|---|---|---|---|---|
| AT-01 | Login del owner | `POST /api/v1/auth/login` | 200 + tokens; `/api/v1/me` OK | ☐ |
| AT-02 | Refresh y reuse detection | Refresh dos veces con el mismo token | Segundo → 401 y familia revocada | ☐ |
| AT-03 | MQTT anónimo | `mosquitto_sub` sin credenciales a 8883 | Conexión rechazada | ☐ |
| AT-04 | Device token | `POST /api/v1/auth/device-token` con secreto correcto/incorrecto | 200 / 401 | ☐ |
| AT-05 | ACL cruzada | Credencial de E intentando `…/{W}/down/#` | Sin mensajes / rechazo | ☐ |
| AT-06 | TLS HTTPS | `testssl.sh https://${JARVIS_DOMAIN}` | Sin vulnerabilidades altas; TLS ≥ 1.2 | ☐ |
| AT-07 | TLS MQTT | `openssl s_client -connect ${JARVIS_DOMAIN}:8883` | Cadena válida Let's Encrypt | ☐ |
| AT-08 | Puertos | `nmap -p- <ip>` desde fuera | Solo 22, 80, 443, 8883 | ☐ |
| AT-09 | Readiness no expuesta | `GET https://${JARVIS_DOMAIN}/health/ready` | 404 | ☐ |
| AT-10 | Rate limit | 20 logins en 1 min | 429 | ☐ |
| AT-11 | Sin secretos en logs | Registrar un device; `docker compose logs` + `grep` del secreto | Sin coincidencias | ☐ |
| AT-12 | Aislamiento entre owners | Segundo owner consulta device/tarea/eventos del primero | 404 / sin datos | ☐ |
| AT-13 | Demo principal | Procedimiento de [MVP Definition §1](../09-backlog/mvp-definition.md#1-demostración-principal-en-el-vps-de-producción) | Todos los pasos OK | ☐ |
| AT-14 | Reinicio del VPS | `sudo reboot` | Servicios `healthy`, simuladores reconectan, tareas intactas | ☐ |
| AT-15 | Restauración de backup | Runbook en entorno aislado | Servicio restaurado; simulador conecta con su credencial | ☐ |
| AT-16 | E2E en CI | Pipeline | E2E-01..E2E-10 en verde | ☐ |
| AT-17 | Independencia de LLM | Búsqueda de `claude\|anthropic\|openai\|llm\|prompt\|model` en `src/` | Sin dependencias ni campos de protocolo relacionados | ☐ |
