# ADR-014 — Inferencia en el borde y vectores calculados por los devices

- **Status:** Accepted · puntos 1 y 5 sustituidos por [ADR-017](ADR-017-server-model-services.md) (modelos en el servidor por defecto)
- **Fecha:** 2026-10-22
- **Precisa:** [ADR-010](ADR-010-llm-independence.md) (sigue vigente: el Cloud no conoce modelos de lenguaje, prompts ni credenciales de IA).

## Context

El encargo pide un *Inference Gateway* para elegir dónde corre el modelo (Ollama local, Ollama remoto, API compatible) y una memoria con búsqueda semántica en PostgreSQL + pgvector. Ambas cosas parecen exigir que el Cloud ejecute o conozca modelos, lo que ADR-010 prohíbe por buenas razones: un VPS comprometido con capacidad de razonamiento y credenciales de IA es un riesgo mayor, y el Cloud quedaría acoplado a un proveedor.

## Decision

1. **El Inference Gateway vive en la app de escritorio.** Es una implementación del contrato `LlmProvider` que elige, por configuración, entre:
   - Ollama local (por defecto);
   - Ollama remoto (URL + token opcional);
   - una API compatible con el formato *chat completions* (URL + modelo + clave guardada en el Credential Manager de Windows).

   Modo `auto`: local si responde; si no, el remoto configurado. El agente no cambia al cambiar de proveedor.
2. **El Cloud no ejecuta modelos.** Los vectores de la memoria los calcula el device que tiene el modelo y los sube junto con:
   - el nombre del modelo (`qwen3-embedding:0.6b`, por ejemplo);
   - su dimensión;
   - el hash del contenido del que salieron.

   El Cloud los guarda en `memory.memory_embeddings`, uno por recuerdo y modelo, y los descarta cuando el contenido cambia.
3. **Búsqueda semántica en el Cloud con el vector de la consulta calculado por el device.** `POST /api/v1/memories/search` recibe texto y, opcionalmente, `{model, vector}`. El Cloud compara solo con vectores del **mismo modelo y dimensión**:
   - si pgvector está instalado, con su operador de distancia coseno;
   - si no, en la aplicación.

   Sin vector, o si nadie subió vectores de ese modelo, usa búsqueda de texto de PostgreSQL. La respuesta indica qué método usó.
4. **Cambio de modelo.** Cambiar de modelo de embeddings no toca el contenido. Cada device recalcula los vectores con su modelo nuevo y los sube; los del modelo anterior pueden coexistir.
5. **Ollama en el VPS es un despliegue opcional, fuera del Cloud Core.** Solo se instala si la inspección del VPS muestra RAM, CPU y disco suficientes (y GPU para modelos de chat medianos). Se publica detrás de nginx con autenticación y nunca recibe herramientas: es «Ollama remoto» para el gateway del escritorio, no lógica del Cloud.

## Alternatives

| Alternativa | Por qué no |
|---|---|
| El Cloud calcula embeddings (Ollama en el VPS llamado por la API) | Convierte al Cloud en dependiente de un modelo y viola ADR-010; si el VPS no tiene recursos, la memoria deja de funcionar |
| Gateway en el Cloud que reenvía prompts a proveedores | Credenciales de IA en el VPS y acoplamiento (rechazado en ADR-010) |
| Solo búsqueda de texto en el Cloud | Pierde la recuperación por significado entre idiomas y sinónimos |
| Columna `vector(N)` de dimensión fija | Ata el esquema a un modelo; cambiar de modelo exigiría una migración |

## Consequences

- (+) Cambiar de modelo de chat o de embeddings no requiere cambios en el Cloud ni en el protocolo.
- (+) El Cloud sigue funcionando aunque pgvector no esté instalado (con menor rendimiento en la búsqueda vectorial).
- (−) Dos devices con modelos distintos no comparten vectores hasta que cada uno calcule los suyos. Mientras tanto, la búsqueda usa texto.
- (−) Con muchos recuerdos y sin índice HNSW, la búsqueda vectorial es lineal. Para la escala personal (decenas de miles) es suficiente. Un índice parcial por dimensión (`hnsw ((embedding::vector(1024)))`) queda documentado como mejora.
