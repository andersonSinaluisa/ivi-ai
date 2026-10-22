# Despliegue en el VPS de Contabo (Cloud Core + memoria)

> Estado: **preparado, no ejecutado.** Nada de este documento se ha aplicado al VPS. Cada paso que cambia el servidor
> requiere la autorización explícita del owner, y se hace a mano siguiendo este orden.

Este documento cubre el despliegue del Cloud Core con los módulos nuevos (memoria, sincronización, experiencias,
habilidades y jobs, [ADR-013](../08-decisions/ADR-013-memory-module-in-cloud-core.md) a
[ADR-016](../08-decisions/ADR-016-worker-and-scheduled-jobs.md)) sobre un VPS que **ya tiene otros servicios**. Complementa los
[Runbooks](runbooks.md); el procedimiento base sigue siendo `deploy/scripts/deploy.sh`.

## Qué se instala y qué no

| Componente | Dónde | Notas |
|---|---|---|
| `jarvis-api` | Docker, red del host, `127.0.0.1:8080/8081` | Sin cambios de forma; ahora incluye `/api/v1/sync`, `/memories`, `/experiences`, `/skills`, `/jobs`, `/health` |
| `jarvis-worker` | Docker, red del host, **sin puertos** | Nuevo. Jobs del catálogo cerrado (ADR-016). Lee los backups en solo lectura |
| `mosquitto` | Docker, `8883` público (TLS) y `1883` en loopback | Sin cambios |
| PostgreSQL | **El existente del host** | Base dedicada `jarvis` y roles `jarvis_*` propios. No se cambia su configuración global ni otras bases |
| pgvector | Opcional | Si el paquete del sistema está instalado se activa **solo** en la base `jarvis`. Si no, la búsqueda por similitud se hace en la aplicación |
| nginx | **El existente del host** | Un sitio propio (`jarvis.conf`) con nombres prefijados `jarvis_`; `nginx -t` antes de recargar y restauración automática si falla |
| Ollama, Qwen, VibeVoice | **No se instalan** | La inferencia sigue en el escritorio. Ver [Inferencia remota opcional](#inferencia-remota-opcional) |

Lo que **nunca** se hace: publicar PostgreSQL, borrar o parar contenedores que no sean `jarvis-*`/`mosquitto` del
proyecto `jarvis`, sobrescribir secretos existentes, cambiar `postgresql.conf`, ejecutar migraciones destructivas, ni
exponer herramientas de Windows (el agente de Windows y el escritorio solo hacen conexiones **salientes**).

## Paso 0 — Inspección de solo lectura (requiere autorización para conectarse)

```bash
scp -r api/deploy usuario@vps:/tmp/jarvis-inspect
ssh usuario@vps 'sudo /tmp/jarvis-inspect/scripts/inspect-vps.sh' > informe-vps.txt
```

`inspect-vps.sh` no instala ni modifica nada (solo consulta; `nginx -t` puede crear un archivo de log vacío si falta).
No lee secretos ni variables de entorno de contenedores. Con el informe se decide:

| Pregunta | Dónde mirar en el informe | Qué hacer |
|---|---|---|
| ¿PostgreSQL es nativo o está en un contenedor? | «PostgreSQL» | Nativo: el plan de abajo. En contenedor: hay que adaptar `deploy.sh` (paso 3) para ejecutar `provision.sql` con `docker exec` y conectar por un puerto en `127.0.0.1`; **no se publica** |
| ¿Versión ≥ 15? | «Servidor» | Requisito del esquema (`NULLS NOT DISTINCT`) |
| ¿pgvector disponible? | «pgvector disponible» | Opcional. Para activarlo: `apt install postgresql-<versión>-pgvector` (cambio del sistema → pedir autorización) |
| ¿`pg_hba.conf` permite contraseña por `127.0.0.1`? | «pg_hba.conf» | Hace falta `host jarvis all 127.0.0.1/32 scram-sha-256`. Si no está, añadir **solo esa línea** y `reload` (no `restart`) |
| ¿Puertos 8080, 8081, 1883, 8883 libres? | «Puertos en escucha» | Si 8080/8081 están ocupados hay que mover `ASPNETCORE_URLS` y el `upstream` de nginx |
| ¿PostgreSQL escucha fuera de loopback? | Aviso en «PostgreSQL» | No lo cambia Jarvis; conviene que el cortafuegos lo bloquee |
| ¿Bases con `CONNECT` a `PUBLIC`? | «Bases que cualquier rol puede abrir» | Valor por defecto de PostgreSQL: los roles `jarvis_*` podrían abrirlas (sin leer tablas). Opcional, decisión del owner: `REVOKE CONNECT ON DATABASE <otra> FROM PUBLIC` |
| ¿nginx ≥ 1.25.1? | «nginx» | Si es menor (Ubuntu 22.04/24.04), `deploy.sh` cambia `http2 on;` por `listen 443 ssl http2;` automáticamente |
| ¿Hay otro proxy en el 80/443 (Traefik, Caddy, nginx en contenedor)? | «nginx» y «Docker» | Entonces no se instala el sitio (`--skip-nginx`) y se añade una ruta equivalente en ese proxy |
| ¿RAM suficiente? | «Sistema» | API ≤ 1 GiB, worker ≤ 384 MiB, Mosquitto ≤ 256 MiB (límites del compose) |

## Paso 1 — Preparar (sin tocar servicios)

1. Traer el código desde GitHub (`andersonSinaluisa/ivi-ai`). Al VPS solo le hace falta la carpeta `api/`: se clona
   con *sparse checkout* y `/opt/jarvis/compose` apunta a ella, así todas las rutas de los [Runbooks](runbooks.md) valen:

   ```bash
   sudo git clone --filter=blob:none --sparse https://github.com/andersonSinaluisa/ivi-ai.git /opt/jarvis/src
   sudo git -C /opt/jarvis/src sparse-checkout set api
   sudo ln -sfn /opt/jarvis/src/api /opt/jarvis/compose
   cd /opt/jarvis/compose
   ```

   Si el repositorio es privado, usar una *deploy key* de solo lectura (`ssh-keygen -t ed25519` en el VPS y añadir la
   pública en GitHub → Settings → Deploy keys) y clonar con `git@github.com:andersonSinaluisa/ivi-ai.git`.
   Actualizar después: `sudo git -C /opt/jarvis/src pull && sudo ./deploy/scripts/deploy.sh`.
2. `cp deploy/.env.example deploy/.env` y rellenar `JARVIS_DOMAIN`, `CERTBOT_EMAIL`, `PG_PORT`, `PG_DATABASE`.
3. DNS: registro `A` del dominio hacia el VPS.
4. Clave de backups: generar el par `age` **fuera** del VPS y copiar solo la pública a `/opt/jarvis/backup-recipient.txt`.

## Paso 2 — Desplegar (requiere autorización)

```bash
sudo ./deploy/scripts/deploy.sh --owner-email <tu-correo>
```

Lo que hace, en orden, y cómo se deshace:

| Paso | Cambio | Reversión |
|---|---|---|
| Secretos | Crea los que falten en `/opt/jarvis/secrets` (nunca sobrescribe). Los de la API/worker pasan a `root:1654 0440` | Borrar el directorio (solo si no hay datos) |
| PostgreSQL | Roles `jarvis_migrator/app/maintenance/backup/worker`, base `jarvis`, esquemas `jarvis` y `memory`, `citext`, `vector` si existe | `DROP DATABASE jarvis; DROP ROLE jarvis_…` (destructivo: solo antes de tener datos) |
| Certificado | Let's Encrypt por webroot | `certbot delete --cert-name <dominio>` |
| Contenedores | `mosquitto`, `jarvis-api`, `jarvis-worker` en el proyecto compose `jarvis` | `docker compose -p jarvis down` (no toca otros proyectos) |
| Migraciones | Solo aditivas (tablas e índices nuevos en `memory`) | Volver al tag anterior; las tablas nuevas no molestan a la versión previa |
| nginx | Sitio `jarvis.conf` (copia de seguridad `.bak` del anterior) | `rm` del sitio + `systemctl reload nginx` |

## Paso 3 — Verificar

```bash
curl -fsS https://<dominio>/api/v1/health          # protocolo jarvis.sync/1 y búsqueda (pgvector o aplicación)
docker compose -p jarvis ps                       # api, worker y mosquitto «healthy»
docker compose -p jarvis logs --tail 50 jarvis-worker
```

Desde el escritorio: Configuración → Infraestructura → «Probar URL», emparejar, «Sincronizar ahora». En el VPS los
recuerdos aparecen en el grafo neuronal del escritorio cuando una búsqueda los trae (nodos violetas).

Jobs útiles para comprobar el worker (todos del catálogo; ninguno ejecuta código arbitrario):

```bash
# con un token de owner
curl -X POST https://<dominio>/api/v1/jobs -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
  -d '{"type":"backup.verify","name":"Verificar backups","scheduleKind":"daily","dailyMinuteUtc":360}'
curl -X POST https://<dominio>/api/v1/jobs -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
  -d '{"type":"http.check","name":"API viva","scheduleKind":"interval","intervalSeconds":900,"parameters":{"target":"api"}}'
```

## Backups

`deploy/scripts/backup.sh` (timer diario, ver [Runbooks](runbooks.md#4-backups)) vuelca la base completa —incluido el
esquema `memory`— cifrada con `age`, escribe `SHA256SUMS` y deja los `.age` legibles para el grupo 1654. El job
`backup.verify` del worker comprueba antigüedad, cabecera `age` y suma; la restauración de prueba se hace fuera del VPS,
donde está la clave privada.

## Inferencia remota opcional (Qwen en el VPS)

El Cloud Core **no** ejecuta modelos: Ollama es un servicio aparte del owner. Si está en el VPS, se publica por el nginx
de Jarvis con TLS y un token, nunca abriendo el puerto 11434:

1. Ollama escucha solo en loopback: `ss -tlnp 'sport = :11434'` debe mostrar `127.0.0.1:11434`. Si muestra `0.0.0.0` o
   `*`, en `systemctl edit ollama` poner `Environment="OLLAMA_HOST=127.0.0.1:11434"` (o publicar el contenedor como
   `127.0.0.1:11434:11434`) y reiniciar Ollama.
2. Ruta con token (la plantilla del sitio incluye `/etc/nginx/jarvis-extra/*.conf`, que `deploy.sh` no sobrescribe):

   ```bash
   TOKEN=$(openssl rand -hex 32); echo "$TOKEN"     # guárdalo: va en la app
   sudo install -d -m 0750 /etc/nginx/jarvis-extra
   sed "s/__TOKEN__/$TOKEN/" deploy/nginx/extra/ollama.conf.example | sudo tee /etc/nginx/jarvis-extra/ollama.conf >/dev/null
   sudo chmod 0640 /etc/nginx/jarvis-extra/ollama.conf
   sudo nginx -t && sudo systemctl reload nginx
   curl -H "Authorization: Bearer $TOKEN" https://<dominio>/ollama/api/tags   # lista los modelos
   ```
3. En la app: Configuración → **Modelos** → «Ollama remoto», URL `https://<dominio>/ollama`, modelo tal como aparece en
   `ollama list`, clave = el token (se guarda en el Administrador de credenciales). Preferencia «Remoto» si la PC no
   tiene Ollama; «Automático» para usar el local y caer al VPS.

Sin GPU, Qwen en el VPS responde despacio (`qwen3:4b` en CPU: varios segundos por respuesta) y compite por RAM con
PostgreSQL. Los embeddings de la memoria siguen calculándose en la PC; sin Ollama local la memoria usa búsqueda léxica.
