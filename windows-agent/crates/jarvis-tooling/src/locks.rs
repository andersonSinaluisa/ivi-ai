//! Resource locks (docs/01-architecture/concurrency-model.md).
//!
//! Todos los locks de una invocación se adquieren en orden canónico (por clave) para evitar
//! deadlocks. `Shared` permite lectores concurrentes; `Exclusive` serializa.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LockMode {
    Shared,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLock {
    pub key: String,
    pub mode: LockMode,
}

impl ResourceLock {
    /// Foco, teclado y ratón del usuario: un único recurso.
    pub const DESKTOP_INPUT: &'static str = "DESKTOP_INPUT";

    #[must_use]
    pub fn exclusive(key: impl Into<String>) -> Self {
        Self { key: key.into(), mode: LockMode::Exclusive }
    }

    #[must_use]
    pub fn shared(key: impl Into<String>) -> Self {
        Self { key: key.into(), mode: LockMode::Shared }
    }

    #[must_use]
    pub fn app(app_id: &str) -> Self {
        Self::exclusive(format!("APP:{app_id}"))
    }
}

#[derive(Debug, Default)]
pub struct LockManager {
    locks: Mutex<HashMap<String, Arc<RwLock<()>>>>,
}

#[derive(Debug)]
enum Guard {
    Read(#[allow(dead_code)] OwnedRwLockReadGuard<()>),
    Write(#[allow(dead_code)] OwnedRwLockWriteGuard<()>),
}

/// Conjunto de locks retenidos; se liberan al soltarse.
#[derive(Debug, Default)]
pub struct LockSet {
    guards: Vec<Guard>,
}

impl LockSet {
    #[must_use]
    pub fn len(&self) -> usize {
        self.guards.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }
}

impl LockManager {
    fn entry(&self, key: &str) -> Arc<RwLock<()>> {
        let mut map = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        map.entry(key.to_owned()).or_default().clone()
    }

    /// Adquiere todos los locks (deduplicados; `Exclusive` gana a `Shared` para la misma clave).
    pub async fn acquire(&self, requested: Vec<ResourceLock>) -> LockSet {
        let mut canonical: BTreeMap<String, LockMode> = BTreeMap::new();
        for lock in requested {
            let mode = canonical.entry(lock.key).or_insert(lock.mode);
            *mode = (*mode).max(lock.mode);
        }
        let mut guards = Vec::with_capacity(canonical.len());
        for (key, mode) in canonical {
            let lock = self.entry(&key);
            guards.push(match mode {
                LockMode::Shared => Guard::Read(lock.read_owned().await),
                LockMode::Exclusive => Guard::Write(lock.write_owned().await),
            });
        }
        LockSet { guards }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_exclusive_key_serializes() {
        let manager = Arc::new(LockManager::default());
        let first = manager.acquire(vec![ResourceLock::app("chrome")]).await;
        let m2 = manager.clone();
        let second = tokio::spawn(async move { m2.acquire(vec![ResourceLock::app("chrome")]).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!second.is_finished(), "second acquisition must wait");
        drop(first);
        let set = tokio::time::timeout(Duration::from_secs(1), second).await.expect("released").expect("joined");
        assert_eq!(set.len(), 1);
    }

    #[tokio::test]
    async fn different_keys_and_shared_locks_run_in_parallel() {
        let manager = LockManager::default();
        let _a = manager.acquire(vec![ResourceLock::app("chrome")]).await;
        let b =
            tokio::time::timeout(Duration::from_millis(200), manager.acquire(vec![ResourceLock::app("notepad")])).await;
        assert!(b.is_ok());
        let _r1 = manager.acquire(vec![ResourceLock::shared("FS:c:/x")]).await;
        let r2 =
            tokio::time::timeout(Duration::from_millis(200), manager.acquire(vec![ResourceLock::shared("FS:c:/x")]))
                .await;
        assert!(r2.is_ok());
    }

    #[tokio::test]
    async fn duplicates_are_merged_with_exclusive_winning() {
        let manager = LockManager::default();
        let set = manager
            .acquire(vec![ResourceLock::shared("K"), ResourceLock::exclusive("K"), ResourceLock::shared("K")])
            .await;
        assert_eq!(set.len(), 1);
        let again =
            tokio::time::timeout(Duration::from_millis(100), manager.acquire(vec![ResourceLock::shared("K")])).await;
        assert!(again.is_err(), "exclusive must block shared");
    }
}
