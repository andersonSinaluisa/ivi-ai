# ADR-017 — Modelos, transcripción y voz en el servidor, con la credencial del device

- **Status:** Accepted · Supersedes en parte [ADR-014](ADR-014-edge-inference-and-device-embeddings.md) (puntos 1 y 5)
- **Fecha:** 2026-10-22
- **Mantiene:** [ADR-010](ADR-010-llm-independence.md): el Cloud Core sigue sin ejecutar ni conocer modelos.

## Context

El owner decidió que **toda la configuración por defecto use el servidor**: Qwen (chat y embeddings), transcripción y
voz, y que la app de escritorio no lleve modelos (se retiran Ollama local, faster-whisper local, OpenVoice y
VibeVoice). Sin servidor, Jarvis no responde ni habla (decisión explícita: «solo servidor»).

ADR-014 ponía el Inference Gateway con Ollama local por defecto y dejaba Ollama en el VPS como opción con token propio.

## Decision

1. **Tres servicios del owner junto al Cloud Core, no dentro de él**, todos escuchando solo en `127.0.0.1`:
   - Ollama (`:11434`): Qwen y `qwen3-embedding`.
   - `jarvis-stt` (`:9100`, faster-whisper en CPU) y `jarvis-tts` (`:9200`, Piper en CPU), en `deploy/voice/`.
   El Cloud Core no los llama ni sabe qué modelos hay (ADR-010 intacto); `deploy.sh` levanta los de voz y comprueba
   que Ollama responda.
2. **nginx los publica en `/ollama/`, `/stt/` y `/tts/` con `auth_request`** a `GET /api/v1/access/check` del Cloud
   Core: la misma credencial que la API (JWT de usuario o device activo). El token no llega al servicio. Revocar el
   device corta el acceso. No hay tokens propios por servicio.
3. **El escritorio usa la credencial del device emparejado**:
   - Rust: el gateway (modo `server`, por defecto) y los embeddings apuntan a `<servidor>/ollama` con un
     `BearerSource` que pide el token del device (renovado con 3 min de margen).
   - Servicio de voz (Python, `127.0.0.1:8765`): conserva micrófono, VAD, palabra de activación y altavoz; recibe la
     URL y el token por `PUT /cloud` desde la app (solo en memoria, renovado cada minuto y ante un 401) y envía cada
     frase a `/stt` y cada respuesta a `/tts`.
4. **Proveedor propio** (modo `custom`: otro Ollama o API compatible con URL y clave en el Credential Manager) se
   mantiene como alternativa explícita; no hay fallback automático entre ellos.
5. Los vectores siguen etiquetados por modelo (`ollama:qwen3-embedding:0.6b`): calcularlos en el servidor produce los
   mismos que antes en la PC, así que no hay que recalcular la memoria.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| Mantener local por defecto con fallback al servidor | El owner quiere una PC ligera, sin modelos ni descargas (la PC además no alcanza Hugging Face) |
| Un token por servicio en nginx | Otro secreto que copiar a mano y que no se revoca con el device |
| Que la API del Cloud Core haga de proxy de los modelos | Mete tráfico de modelos y su conocimiento en el Cloud Core (ADR-010) |
| Servicios de voz de terceros ya hechos (speaches) | Imágenes en versiones candidatas y descargas manuales de modelos; dos servidores mínimos propios son más fáciles de auditar |

## Consequences

- (+) La PC no descarga ni ejecuta modelos; emparejarla deja todo configurado.
- (+) Revocar un device corta también modelos y voz.
- (−) Sin conexión no hay chat ni voz (la memoria y el historial locales se conservan y se sincronizan al volver).
- (−) Cada respuesta añade la latencia de red; en un VPS sin GPU, Qwen y Whisper van en CPU.
- (−) El audio de cada frase y el texto de cada respuesta viajan al servidor del owner (cifrados en tránsito; los
  servicios no los guardan).
- (−) Piper es GPL-3.0: se ejecuta como servicio aparte sin modificarlo; las voces tienen su propia licencia.
