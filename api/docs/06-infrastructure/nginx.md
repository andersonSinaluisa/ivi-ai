# Nginx

> **Producción (ADR-011):** PostgreSQL y nginx son **nativos** en el host; Docker solo ejecuta `jarvis-api` (red del host, escucha en 127.0.0.1) y `mosquitto`. Ver [ADR-011](../08-decisions/ADR-011-native-postgres-nginx.md) y [Runbooks](runbooks.md). Las secciones que describen nginx/PostgreSQL en contenedores quedan como referencia del diseño original y siguen aplicando a desarrollo/pruebas.

Rol: terminación TLS para HTTPS/WSS, reverse proxy a `jarvis-api:8080`, rate limiting perimetral, headers de seguridad y servidor del desafío ACME. **No** hace proxy de MQTT (Mosquitto termina su propio TLS en 8883).

Dominio: placeholder `${JARVIS_DOMAIN}` (un único nombre para HTTPS y MQTT → un solo certificado).

## Server blocks

| Server | Escucha | Función |
|---|---|---|
| HTTP | `80` | `/.well-known/acme-challenge/` desde `/var/www/certbot`; `/nginx-health` solo desde `127.0.0.1`; todo lo demás `301` a HTTPS |
| HTTPS | `443 ssl` + `http2` | Proxy a la API |
| Default | `443 ssl default_server` | Cierra conexiones con `Host` desconocido (`return 444`), evita exposición a escaneos por IP |

## Locations (HTTPS)

| Location | Destino | Rate limit | Timeouts | Notas |
|---|---|---|---|---|
| `= /health/live` | api | general | 5 s | |
| `^~ /health/` (resto) | — | — | — | `return 404` (readiness solo interna) |
| `^~ /api/v1/auth/` | api | `auth` 10 r/min burst 5 nodelay | 15 s | `client_max_body_size 4k`; log sin cuerpo |
| `^~ /api/` | api | `api` 20 r/s burst 40 | read 30 s | `client_max_body_size 64k` |
| `^~ /hubs/` | api | `limit_conn hubconn 10` | read/send 3600 s | WebSocket: `proxy_http_version 1.1`, `Upgrade $http_upgrade`, `Connection $connection_upgrade`, `proxy_buffering off`; log **sin query string** |
| `/` | — | — | — | `return 404` |

Headers de proxy: `Host`, `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Request-Id $request_id` (la API lo usa como correlation id si el cliente no envía `X-Correlation-Id`). Se **eliminan** del request del cliente cualquier `X-Forwarded-*` previo.

## TLS

| Parámetro | Valor |
|---|---|
| Protocolos | `TLSv1.2 TLSv1.3` |
| Cifrados | Perfil "intermediate" de Mozilla |
| `ssl_session_tickets` | `off` |
| OCSP stapling | `off`: Let's Encrypt retiró su servicio OCSP en 2025; la revocación se apoya en CRLs y certificados de vida corta |
| Certificado | `/etc/letsencrypt/live/${JARVIS_DOMAIN}/fullchain.pem` + `privkey.pem` |

## Headers de respuesta

```text
Strict-Transport-Security: max-age=31536000; includeSubDomains
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Referrer-Policy: no-referrer
Content-Security-Policy: default-src 'none'; frame-ancestors 'none'
Permissions-Policy: ()
```

`server_tokens off`. `proxy_hide_header X-Powered-By`.

## Rate limit zones

```text
limit_req_zone  $binary_remote_addr zone=auth:10m rate=10r/m;
limit_req_zone  $binary_remote_addr zone=api:10m  rate=20r/s;
limit_conn_zone $binary_remote_addr zone=hubconn:10m;
limit_req_status 429; limit_conn_status 429;
```

## Límites generales

`client_body_timeout 10s`, `client_header_timeout 10s`, `keepalive_timeout 30s`, `large_client_header_buffers 4 8k`, `client_max_body_size 64k` por defecto.

## Logs

Formato JSON propio que **excluye** query strings y el header `Authorization`:

```text
log_format jarvis escape=json '{"time":"$time_iso8601","ip":"$remote_addr","method":"$request_method","path":"$uri","status":$status,"bytes":$body_bytes_sent,"rt":$request_time,"rid":"$request_id","ua":"$http_user_agent"}';
```

Se usa `$uri` en lugar de `$request` para no registrar `?access_token=` de SignalR.

## Let's Encrypt

Método: **webroot** con certbot en el host.

Emisión inicial (bootstrap: nginx aún no tiene certificado):
1. Arrancar nginx con una configuración solo-HTTP (server 80 con el location ACME).
2. `certbot certonly --webroot -w /var/www/certbot -d ${JARVIS_DOMAIN} --email <owner> --agree-tos --no-eff-email`.
3. Activar la configuración completa y recargar nginx.

Renovación: timer systemd de certbot (2 veces al día). `--deploy-hook` ejecuta:
- `docker compose exec nginx nginx -s reload`
- `docker compose restart mosquitto` (Mosquitto lee los certificados al iniciar; un restart provoca reconexión de devices, aceptable con la frecuencia de renovación; Let's Encrypt está reduciendo la vida de los certificados hacia 45 días, así que la automatización es obligatoria. Si la versión desplegada soporta recarga de certificados con `SIGHUP`, preferir `docker compose kill -s HUP mosquitto` y verificarlo en la HU).

Permisos: Mosquitto corre como usuario `mosquitto` y necesita leer `privkey.pem`. El deploy-hook copia `fullchain.pem`/`privkey.pem` a `/opt/jarvis/config/mosquitto/certs/` con propietario UID de mosquitto y modo 0400, en lugar de relajar permisos de `/etc/letsencrypt`. Mosquitto monta ese directorio en `/mosquitto/certs` ([Docker Strategy](docker-strategy.md)).

Monitoreo: alerta si el certificado expira en < 14 días (ver [Observability](observability.md)).
