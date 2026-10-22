//! Proveedores de embeddings.
//!
//! - `OllamaEmbedder`: semántico, local (por defecto `qwen3-embedding:0.6b`, multilingüe).
//! - `HashingEmbedder`: respaldo **léxico** sin modelo (palabras y trigramas con hashing). Sirve para
//!   que la memoria funcione sin descargas, pero no entiende sinónimos: la UI lo indica.
//!
//! Cada vector se guarda con el identificador del modelo. Si el modelo cambia, los vectores viejos
//! se ignoran y se recalculan (`MemoryEngine::reembed`).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum EmbeddingError {
    #[error("proveedor de embeddings no disponible: {0}")]
    Unavailable(String),
    #[error("el modelo de embeddings '{0}' no está instalado (ollama pull {0})")]
    ModelNotFound(String),
    #[error("respuesta inválida del proveedor de embeddings: {0}")]
    Protocol(String),
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Identificador estable que se guarda con cada vector (incluye el proveedor).
    fn model_id(&self) -> String;
    /// `false` para el respaldo léxico.
    fn is_semantic(&self) -> bool;
    /// Similitud mínima para considerar que un recuerdo es pertinente.
    fn min_similarity(&self) -> f32;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
}

/// Normaliza a longitud 1 (la similitud coseno pasa a ser un producto escalar).
pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

// ---------------------------------------------------------------------- respaldo léxico

pub struct HashingEmbedder {
    dims: usize,
}

impl HashingEmbedder {
    #[must_use]
    pub fn new(dims: usize) -> Self {
        Self { dims: dims.max(32) }
    }
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self::new(384)
    }
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        other => other,
    }
}

/// Palabras vacías frecuentes en español e inglés.
const STOPWORDS: &[&str] = &[
    "el", "la", "los", "las", "un", "una", "de", "del", "que", "y", "o", "en", "a", "al", "es", "por", "para", "con",
    "mi", "me", "se", "su", "lo", "le", "como", "mas", "pero", "the", "and", "of", "to", "is", "in", "que", "cual",
    "esta", "este", "eso", "esa", "yo", "tu", "te",
];

pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let folded: String = text.to_lowercase().chars().map(fold).collect();
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2 && !STOPWORDS.contains(w))
        .map(str::to_owned)
        .collect()
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl HashingEmbedder {
    fn embed_one(&self, text: &str) -> Vec<f32> {
        // Frecuencia sublineal (1 + ln tf): una palabra repetida no ahoga al resto.
        let mut features: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let mut add = |feature: String, weight: f32| *features.entry(feature).or_insert(0.0) += weight;
        for word in tokenize(text) {
            add(format!("w:{word}"), 1.0);
            // Prefijo como "raíz" barata: proyecto/proyectos, recordar/recuerdo.
            let prefix: String = word.chars().take(5).collect();
            if prefix.chars().count() >= 4 {
                add(format!("p:{prefix}"), 0.7);
            }
            let chars: Vec<char> = format!("^{word}$").chars().collect();
            for tri in chars.windows(3) {
                add(format!("t:{}", tri.iter().collect::<String>()), 0.25);
            }
        }
        let mut v = vec![0.0f32; self.dims];
        for (feature, tf) in features {
            let h = fnv1a(&feature);
            #[allow(clippy::cast_possible_truncation)]
            let idx = (h % self.dims as u64) as usize;
            let sign = if (h >> 63) & 1 == 1 { -1.0 } else { 1.0 };
            v[idx] += sign * (1.0 + tf.ln_1p());
        }
        normalize(&mut v);
        v
    }
}

#[async_trait]
impl EmbeddingProvider for HashingEmbedder {
    fn model_id(&self) -> String {
        format!("lexical-hash-v1:{}", self.dims)
    }

    fn is_semantic(&self) -> bool {
        false
    }

    fn min_similarity(&self) -> f32 {
        0.12
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

// ---------------------------------------------------------------------- Ollama

pub struct OllamaEmbedder {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaEmbedder {
    /// # Errors
    /// Si no se puede construir el cliente HTTP.
    pub fn new(base_url: &str, model: &str) -> Result<Self, EmbeddingError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| EmbeddingError::Protocol(e.to_string()))?;
        Ok(Self { client, base_url: base_url.trim_end_matches('/').to_owned(), model: model.to_owned() })
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedder {
    fn model_id(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn is_semantic(&self) -> bool {
        true
    }

    fn min_similarity(&self) -> f32 {
        0.3
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&json!({"model": self.model, "input": texts}))
            .send()
            .await
            .map_err(|e| EmbeddingError::Unavailable(e.to_string()))?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|e| EmbeddingError::Protocol(e.to_string()))?;
        if status.as_u16() == 404 {
            return Err(EmbeddingError::ModelNotFound(self.model.clone()));
        }
        if !status.is_success() {
            let msg = body.get("error").and_then(Value::as_str).unwrap_or("error desconocido");
            if msg.contains("not found") {
                return Err(EmbeddingError::ModelNotFound(self.model.clone()));
            }
            return Err(EmbeddingError::Unavailable(format!("HTTP {status}: {msg}")));
        }
        let vectors: Vec<Vec<f32>> = serde_json::from_value(body.get("embeddings").cloned().unwrap_or(Value::Null))
            .map_err(|e| EmbeddingError::Protocol(format!("falta 'embeddings': {e}")))?;
        if vectors.len() != texts.len() {
            return Err(EmbeddingError::Protocol(format!("pedí {} vectores y llegaron {}", texts.len(), vectors.len())));
        }
        Ok(vectors
            .into_iter()
            .map(|mut v| {
                normalize(&mut v);
                v
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn lexical_fallback_ranks_related_text_higher() {
        let e = HashingEmbedder::default();
        let v = e
            .embed(&[
                "Mi proyecto LOOP usa FastAPI y React".into(),
                "¿Qué tecnologías usa el proyecto LOOP?".into(),
                "Me gusta el café de Loja por la mañana".into(),
            ])
            .await
            .expect("ok");
        let related = cosine(&v[0], &v[1]);
        let unrelated = cosine(&v[0], &v[2]);
        assert!(related > unrelated + 0.1, "related={related} unrelated={unrelated}");
        assert!(related > e.min_similarity());
        assert!((v[0].iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4, "normalizado");
        // Repetir una palabra no ahoga la coincidencia con el resto.
        let r = e.embed(&["proyecto LOOP".into(), format!("Dato del proyecto LOOP: {}", "detalle largo ".repeat(20))]).await.expect("ok");
        assert!(cosine(&r[0], &r[1]) > e.min_similarity(), "{}", cosine(&r[0], &r[1]));
    }

    #[tokio::test]
    async fn ollama_embeddings_are_parsed_and_normalized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/embed"))
            .and(body_partial_json(json!({"model": "qwen3-embedding:0.6b"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"embeddings": [[3.0, 4.0], [0.0, 2.0]]})))
            .mount(&server)
            .await;
        let e = OllamaEmbedder::new(&server.uri(), "qwen3-embedding:0.6b").expect("e");
        let v = e.embed(&["a".into(), "b".into()]).await.expect("ok");
        assert_eq!(v, vec![vec![0.6, 0.8], vec![0.0, 1.0]]);
        assert_eq!(e.model_id(), "ollama:qwen3-embedding:0.6b");
    }

    #[tokio::test]
    async fn missing_model_and_down_server_are_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/embed"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "model \"x\" not found"})))
            .mount(&server)
            .await;
        let e = OllamaEmbedder::new(&server.uri(), "x").expect("e");
        assert_eq!(e.embed(&["a".into()]).await.expect_err("404"), EmbeddingError::ModelNotFound("x".into()));
        let down = OllamaEmbedder::new("http://127.0.0.1:9", "x").expect("e");
        assert!(matches!(down.embed(&["a".into()]).await, Err(EmbeddingError::Unavailable(_))));
    }
}
