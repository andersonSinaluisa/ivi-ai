# Despliegue en el VPS de Contabo (Cloud Core + memoria)

> Estado: desplegado por el owner en `jarvis.brix-dev.com` (oct. 2026). Cada paso que cambia el servidor se hace a
> mano siguiendo este orden.

Este documento cubre el despliegue del Cloud Core con los módulos nuevos (memoria, sincronización, experiencias,
habilidades y jobs, [ADR-013](../08-decisions/ADR-013-memory-module-in-cloud-core.md) a
[ADR-016](../08-decisions/ADR-016-worker-and-scheduled-jobs.md)) sobre un VPS que **ya tiene otros servicios**. Complementa los
[Runbooks](runbooks.md); el procedimiento base sigue siendo `deploy/scripts/deploy.sh`.

## Qué se instala y qué no

| Componente | Dónde | Notas |
|---|---|---|
| `jarvis-api` | Docker, red del host, `127.0.0.1:${JARVIS_API_PORT:-8080}` y métricas en `${JARVIS_METRICS_PORT:-8081}` | Sin cambios de forma; ahora incluye `/api/v1/sync`, `/memories`, `/experiences`, `/skills`, `/jobs`, `/health` |
| `jarvis-worker` | Docker, red del host, **sin puertos** | Nuevo. Jobs del catálogo cerrado (ADR-016). Lee los backups en solo lectura |
| `mosquitto` | Docker, `8883` público (TLS) y `1883` en loopback | Sin cambios |
| PostgreSQL | **El existente del host** | Base dedicada `jarvis` y roles `jarvis_*` propios. No se cambia su configuración global ni otras bases |
| pgvector | Opcional | Si el paquete del sistema está instalado se activa **solo** en la base `jarvis`. Si no, la búsqueda por similitud se hace en la aplicación |
| nginx | **El existente del host** | Un sitio propio (`jarvis.conf`) con nombres prefijados `jarvis_`; `nginx -t` antes de recargar y restauración automática si falla |
| `jarvis-stt` / `jarvis-tts` | Docker (`deploy/voice`), red del host, `127.0.0.1:${JARVIS_STT_PORT:-9100}` / `${JARVIS_TTS_PORT:-9200}` (en `deploy/.env` si están ocupados) | Transcripción (faster-whisper) y voz (Piper) en CPU. Ver [Servicios de modelos](#servicios-de-modelos-y-voz-adr-017) |
| Ollama (Qwen y embeddings) | **Del owner** (no lo instala `deploy.sh`) | Debe responder en `127.0.0.1:11434`; nginx lo publica en `/ollama/` |

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

## Servicios de modelos y voz (ADR-017)

En la PC no corre ningún modelo: Qwen, los embeddings de la memoria, la transcripción y la voz están en el servidor.
El Cloud Core **no** los ejecuta (ADR-010): son procesos aparte en `127.0.0.1` que nginx publica exigiendo la **misma
credencial que la API** (`auth_request` a `/api/v1/access/check`). Al emparejar la PC queda todo configurado: no hay
tokens que copiar, y revocar el equipo le corta también modelos y voz.

| Ruta pública | Servicio local | Quién lo levanta |
|---|---|---|
| `/ollama/` | Ollama en `127.0.0.1:11434` | El owner (ver abajo) |
| `/stt/` | `jarvis-stt` en `127.0.0.1:$JARVIS_STT_PORT` (9100) | `deploy.sh` (`deploy/voice/compose.yml`) |
| `/tts/` | `jarvis-tts` en `127.0.0.1:$JARVIS_TTS_PORT` (9200) | `deploy.sh` (`deploy/voice/compose.yml`) |

### Ollama

Debe escuchar solo en loopback (`ss -tlnp 'sport = :11434'` → `127.0.0.1:11434`) y tener los modelos:

```bash
ollama pull qwen3:4b                 # o el modelo elegido en Configuración → Modelo
ollama pull qwen3-embedding:0.6b     # memoria por significado (sin él, búsqueda por palabras)
```

Si Ollama vive en un contenedor de otro proyecto (p. ej. `loop-ollama-1` en la red `loop_loop`), sin publicar puertos
ni tocar ese proyecto se expone en loopback con un reenviador:

```bash
sudo docker run -d --name jarvis-ollama-proxy --restart unless-stopped \
  --network loop_loop -p 127.0.0.1:11434:11434 \
  --read-only --cap-drop ALL --security-opt no-new-privileges:true --memory 32m \
  alpine/socat tcp-listen:11434,fork,reuseaddr tcp-connect:ollama:11434
sudo docker exec loop-ollama-1 ollama pull qwen3-embedding:0.6b
```

### Voz

`deploy.sh` construye y levanta `jarvis-stt` y `jarvis-tts` (omitir con `--skip-voice`). La primera vez descargan
sus modelos en volúmenes (`base` ≈ 145 MB; voz `es_MX-claude-high` ≈ 110 MB):

```bash
sudo docker compose -f /opt/jarvis/compose/deploy/voice/compose.yml logs -f      # progreso de la descarga
curl -s http://127.0.0.1:${JARVIS_STT_PORT:-9100}/health; echo                    # "ready": true
curl -s http://127.0.0.1:${JARVIS_TTS_PORT:-9200}/health; echo
```

| Variable (`deploy/voice/.env`) | Por defecto | Notas |
|---|---|---|
| `WHISPER_MODEL` | `base` | `tiny` más rápido; `small` más preciso y ~3× más lento en CPU |
| `PIPER_VOICE` | `es_MX-claude-high` | Otras: `es_MX-ald-medium`, `es_ES-davefx-medium` |
| `STT_MEM_LIMIT` / `STT_CPUS` | `1500m` / `2.0` | Límites para no competir con PostgreSQL |
| `TTS_MEM_LIMIT` / `TTS_CPUS` | `768m` / `1.0` | |

### Comprobar desde fuera

```bash
TOKEN=$(curl -s -X POST https://<dominio>/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email":"<owner>","password":"<contraseña>"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["accessToken"])')
curl -s -o /dev/null -w '%{http_code}\n' https://<dominio>/ollama/api/tags          # 401 sin credencial
curl -s -H "Authorization: Bearer $TOKEN" https://<dominio>/ollama/api/tags           # modelos
curl -s -H "Authorization: Bearer $TOKEN" https://<dominio>/stt/health
curl -s -H "Authorization: Bearer $TOKEN" https://<dominio>/tts/health
```

Si antes se configuraron rutas con token propio en `/etc/nginx/jarvis-extra/` (`ollama.conf`, `stt.conf`),
`deploy.sh` las renombra a `.disabled` porque chocarían con las de la plantilla.

Privacidad: el audio de cada frase y el texto de cada respuesta viajan de la PC a tu servidor (cifrados en tránsito);
los servicios no los guardan ni registran su contenido. Piper es GPL-3.0 (se ejecuta sin modificar como servicio
aparte); cada voz tiene su licencia en su ficha de `rhasspy/piper-voices`.
