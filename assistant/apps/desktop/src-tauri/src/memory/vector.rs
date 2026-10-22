//! Índice vectorial intercambiable.
//!
//! `InMemoryVectorIndex` es una búsqueda exacta por coseno, suficiente para miles de recuerdos.
//! Para escalar se puede sustituir por sqlite-vec, LanceDB o similar implementando el mismo trait.
//! Aún no está conectado al flujo del agente (falta el proveedor de embeddings; ver PROGRESS.md).

use std::collections::HashMap;
use std::sync::RwLock;

pub trait VectorIndex: Send + Sync {
    fn upsert(&self, id: &str, embedding: Vec<f32>);
    fn remove(&self, id: &str);
    /// Los `k` más parecidos, con su similitud coseno.
    fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)>;
}

#[derive(Default)]
pub struct InMemoryVectorIndex {
    items: RwLock<HashMap<String, Vec<f32>>>,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
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

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&self, id: &str, embedding: Vec<f32>) {
        if let Ok(mut items) = self.items.write() {
            items.insert(id.to_owned(), embedding);
        }
    }

    fn remove(&self, id: &str) {
        if let Ok(mut items) = self.items.write() {
            items.remove(id);
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        let Ok(items) = self.items.read() else { return Vec::new() };
        let mut scored: Vec<(String, f32)> = items.iter().map(|(id, v)| (id.clone(), cosine(query, v))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_nearest_vectors() {
        let idx = InMemoryVectorIndex::default();
        idx.upsert("a", vec![1.0, 0.0]);
        idx.upsert("b", vec![0.0, 1.0]);
        idx.upsert("c", vec![0.9, 0.1]);
        let r = idx.search(&[1.0, 0.0], 2);
        assert_eq!(r[0].0, "a");
        assert_eq!(r[1].0, "c");
        idx.remove("a");
        assert_eq!(idx.search(&[1.0, 0.0], 1)[0].0, "c");
    }
}
