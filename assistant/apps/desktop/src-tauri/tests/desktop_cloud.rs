//! Integración Escritorio ↔ Cloud Core real (escenarios A, B, C, D, F, G y H).
//!
//! Necesita un Cloud Core corriendo (`ivi/tests/desktop-cloud/run-local.sh` lo levanta con PostgreSQL y Mosquitto
//! nativos) y estas variables: `JARVIS_CLOUD_IT_URL`, `JARVIS_CLOUD_IT_EMAIL`, `JARVIS_CLOUD_IT_PASSWORD`.
//! Sin ellas, las pruebas terminan sin hacer nada (y lo dicen).

use std::sync::Arc;

use async_trait::async_trait;
use jarvis_desktop_lib::domain::memory::{MemoryKind, MemoryPatch, NewMemory};
use jarvis_desktop_lib::infrastructure::persistence::Database;
use jarvis_desktop_lib::memory::embedding_provider::{EmbeddingError, EmbeddingProvider, HashingEmbedder};
use jarvis_desktop_lib::memory::memory_retriever::RetrievalOptions;
use jarvis_desktop_lib::memory::{MemoryConfig, MemoryEngine};
use jarvis_desktop_lib::sync::client::CloudError;
use jarvis_desktop_lib::sync::secrets::{MemoryVault, SecretVault};
use jarvis_desktop_lib::sync::{SyncEngine, SyncMode};

struct Cloud {
    url: String,
    email: String,
    password: String,
}

fn cloud() -> Option<Cloud> {
    let url = std::env::var("JARVIS_CLOUD_IT_URL").ok()?;
    Some(Cloud {
        url,
        email: std::env::var("JARVIS_CLOUD_IT_EMAIL").expect("JARVIS_CLOUD_IT_EMAIL"),
        password: std::env::var("JARVIS_CLOUD_IT_PASSWORD").expect("JARVIS_CLOUD_IT_PASSWORD"),
    })
}

macro_rules! require_cloud {
    () => {
        match cloud() {
            Some(c) => c,
            None => {
                eprintln!("JARVIS_CLOUD_IT_URL no definida: prueba de integración con el Cloud Core omitida");
                return;
            }
        }
    };
}

/// Embedder determinista "semántico" de pruebas: cada modelo produce vectores distintos para el mismo texto.
struct FakeSemantic {
    model: &'static str,
}

#[async_trait]
impl EmbeddingProvider for FakeSemantic {
    fn model_id(&self) -> String {
        self.model.into()
    }

    fn is_semantic(&self) -> bool {
        true
    }

    fn min_similarity(&self) -> f32 {
        0.1
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut out = HashingEmbedder::default().embed(texts).await?;
        if self.model.ends_with('b') {
            for v in &mut out {
                v.reverse();
            }
        }
        Ok(out)
    }
}

/// Una "PC": base SQLite propia, almacén de secretos propio y motor de sincronización.
struct Pc {
    memory: Arc<MemoryEngine>,
    sync: Arc<SyncEngine>,
    vault: Arc<MemoryVault>,
    _dir: tempfile::TempDir,
}

impl Pc {
    fn new(embedder: Arc<dyn EmbeddingProvider>) -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let db = Database::open(&dir.path().join("jarvis.db")).expect("db");
        let memory = Arc::new(MemoryEngine::new(db, embedder, MemoryConfig::in_dir(dir.path())).expect("memory"));
        let vault = Arc::new(MemoryVault::default());
        let sync = SyncEngine::new(memory.clone(), vault.clone());
        Self { memory, sync, vault, _dir: dir }
    }

    /// Otra instancia sobre la misma base (reinicio de la app, o la misma PC con otra URL).
    fn reopen(&self) -> Arc<SyncEngine> {
        SyncEngine::new(self.memory.clone(), self.vault.clone() as Arc<dyn SecretVault>)
    }

    async fn pair(&self, cloud: &Cloud, name: &str) {
        let unique = format!("{name}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        self.sync.pair(&cloud.url, &cloud.email, &cloud.password, &unique).await.expect("emparejar");
    }

    async fn remember(&self, text: &str) -> String {
        self.memory.remember(NewMemory::user(MemoryKind::Semantic, text)).await.expect("recordar").memory.id
    }

    async fn sync(&self) -> jarvis_desktop_lib::sync::SyncReport {
        self.sync.run_once().await.expect("sincronizar")
    }

    fn content(&self, id: &str) -> Option<String> {
        self.memory.repository().get(id).expect("get").map(|m| m.content)
    }
}

async fn owner_token(cloud: &Cloud) -> String {
    let response: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/login", cloud.url))
        .json(&serde_json::json!({ "email": cloud.email, "password": cloud.password }))
        .send()
        .await
        .expect("login")
        .json()
        .await
        .expect("json");
    response["accessToken"].as_str().expect("token").to_owned()
}

/// A: un recuerdo creado en la PC A se recupera en la PC B (local tras el pull y también en el Cloud).
#[tokio::test]
async fn scenario_a_memory_created_on_pc_a_is_recovered_on_pc_b() {
    let cloud = require_cloud!();
    let a = Pc::new(Arc::new(HashingEmbedder::default()));
    a.pair(&cloud, "pc-a").await;
    let id = a.remember("El VPS de Contabo usa PostgreSQL 16 y Nginx").await;
    let report = a.sync().await;
    assert!(report.pushed >= 1, "{report:?}");
    assert_eq!(a.sync.status().pending, 0);

    let b = Pc::new(Arc::new(HashingEmbedder::default()));
    b.pair(&cloud, "pc-b").await;
    let pulled = b.sync().await;
    assert!(pulled.memories_changed >= 1, "{pulled:?}");
    assert_eq!(b.content(&id).as_deref(), Some("El VPS de Contabo usa PostgreSQL 16 y Nginx"));

    let found = b.memory.retrieve("¿qué base de datos usa el VPS?", &RetrievalOptions::default()).await.expect("recuperar");
    assert!(found.items.iter().any(|m| m.memory.id == id), "recuperado localmente en B");
    let remote = b.sync.remote_search("PostgreSQL Contabo", None, 5).await.expect("búsqueda en el Cloud");
    assert!(remote.memories.iter().any(|m| m.id == id));
}

/// B: trabajar sin Internet y sincronizar después.
#[tokio::test]
async fn scenario_b_offline_work_is_synchronized_later() {
    let cloud = require_cloud!();
    let a = Pc::new(Arc::new(HashingEmbedder::default()));
    a.pair(&cloud, "pc-offline").await;
    let kept = a.remember("Dato previo a la desconexión").await;
    let doomed = a.remember("Dato que se borrará sin conexión").await;
    a.sync().await;

    // Sin conexión: la misma PC apunta a un puerto cerrado.
    let store_url = cloud.url.clone();
    jarvis_desktop_lib::sync::store::SyncStore::new(a.memory.database().clone())
        .set_meta("base_url", "http://127.0.0.1:9")
        .expect("meta");
    let offline = a.reopen();
    let created = a.remember("Nota escrita sin Internet").await;
    a.memory.update_memory(&kept, MemoryPatch { content: Some("Dato previo, editado sin Internet".into()), ..Default::default() }).await.expect("editar");
    a.memory.delete_memory(&doomed).expect("borrar");
    let error = offline.run_once().await.expect_err("sin conexión");
    assert!(matches!(error, CloudError::Unreachable(_)), "{error:?}");
    assert!(offline.status().pending >= 3);
    let local = a.memory.retrieve("nota escrita sin Internet", &RetrievalOptions::default()).await.expect("local");
    assert!(local.items.iter().any(|m| m.memory.id == created), "Jarvis sigue funcionando sin Cloud");

    // Vuelve la conexión: se vacía el outbox y otra PC recibe todo, incluido el borrado.
    jarvis_desktop_lib::sync::store::SyncStore::new(a.memory.database().clone()).set_meta("base_url", &store_url).expect("meta");
    let online = a.reopen();
    let report = online.run_once().await.expect("sincronizar");
    assert!(report.pushed >= 3, "{report:?}");
    assert_eq!(online.status().pending, 0);

    let b = Pc::new(Arc::new(HashingEmbedder::default()));
    b.pair(&cloud, "pc-b").await;
    b.sync().await;
    assert_eq!(b.content(&created).as_deref(), Some("Nota escrita sin Internet"));
    assert_eq!(b.content(&kept).as_deref(), Some("Dato previo, editado sin Internet"));
    assert_eq!(b.content(&doomed), None);
}

/// C: el mismo recuerdo modificado en dos PCs: campos distintos se fusionan; el mismo campo queda en conflicto
/// hasta que se resuelve, y nada se pierde.
#[tokio::test]
async fn scenario_c_concurrent_edits_merge_or_conflict_without_losing_data() {
    let cloud = require_cloud!();
    let a = Pc::new(Arc::new(HashingEmbedder::default()));
    let b = Pc::new(Arc::new(HashingEmbedder::default()));
    a.pair(&cloud, "pc-a").await;
    b.pair(&cloud, "pc-b").await;
    let id = a.remember("Reunión semanal los lunes").await;
    a.sync().await;
    b.sync().await;

    // Campos distintos: A cambia el contenido, B la confianza.
    a.memory.update_memory(&id, MemoryPatch { content: Some("Reunión semanal los martes".into()), ..Default::default() }).await.expect("a");
    b.memory.update_memory(&id, MemoryPatch { confidence: Some(0.4), ..Default::default() }).await.expect("b");
    a.sync().await;
    let merged = b.sync().await;
    assert_eq!(merged.merged, 1, "{merged:?}");
    assert_eq!(b.content(&id).as_deref(), Some("Reunión semanal los martes"));
    a.sync().await;
    let a_record = a.memory.repository().get(&id).expect("g").expect("m");
    assert!((a_record.confidence - 0.4).abs() < 1e-4);

    // Mismo campo: conflicto. B conserva su versión y la UI lo muestra.
    a.memory.update_memory(&id, MemoryPatch { content: Some("Reunión los miércoles a las 9".into()), ..Default::default() }).await.expect("a");
    b.memory.update_memory(&id, MemoryPatch { content: Some("Reunión los miércoles a las 10".into()), ..Default::default() }).await.expect("b");
    a.sync().await;
    let report = b.sync().await;
    assert_eq!(report.conflicts, 1, "{report:?}");
    assert_eq!(b.content(&id).as_deref(), Some("Reunión los miércoles a las 10"));
    let conflicts = b.sync.conflicts().expect("conflictos");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].fields, vec!["content".to_owned()]);
    assert_eq!(conflicts[0].server_data.as_ref().expect("server")["content"], "Reunión los miércoles a las 9");

    // Se resuelve con un texto nuevo; ambas PCs terminan iguales.
    let mut data = conflicts[0].device_data.clone().expect("device");
    data["content"] = serde_json::json!("Reunión los miércoles a las 9:30");
    b.sync.resolve_conflict(&conflicts[0].id, "custom", Some(data)).await.expect("resolver");
    a.sync().await;
    assert_eq!(b.content(&id).as_deref(), Some("Reunión los miércoles a las 9:30"));
    assert_eq!(a.content(&id).as_deref(), Some("Reunión los miércoles a las 9:30"));
    assert!(b.sync.conflicts().expect("c").is_empty());
}

/// D: cambiar el modelo (de embeddings) sin perder recuerdos; los vectores nuevos viajan y otra PC con el mismo
/// modelo los recibe sin recalcularlos.
#[tokio::test]
async fn scenario_d_changing_the_model_keeps_every_memory() {
    let cloud = require_cloud!();
    let a = Pc::new(Arc::new(FakeSemantic { model: "it-embed-a" }));
    a.pair(&cloud, "pc-model").await;
    let ids = [a.remember("Mi lenguaje favorito es Rust").await, a.remember("Uso Tauri para la interfaz").await];
    a.sync().await;

    // Cambio de modelo: el contenido no se toca; se recalculan y suben los vectores del modelo nuevo.
    a.memory.set_embedder(Arc::new(FakeSemantic { model: "it-embed-b" }), "pruebas".into());
    let cancel = tokio_util::sync::CancellationToken::new();
    // (≥ 2: la misma cuenta del Cloud tiene recuerdos de los otros escenarios, que esta PC también recibió).
    assert!(a.memory.reembed(&cancel).await.expect("reembed") >= 2);
    let report = a.sync().await;
    assert!(report.pushed >= 2, "{report:?}");
    for id in &ids {
        assert!(a.content(id).is_some());
    }

    let query = FakeSemantic { model: "it-embed-b" }.embed(&["lenguaje favorito Rust".into()]).await.expect("q").remove(0);
    let found = a.sync.remote_search("lenguaje favorito", Some(("it-embed-b".into(), query)), 5).await.expect("búsqueda");
    assert!(found.method.starts_with("vector:"), "{}", found.method);
    assert!(found.memories.iter().any(|m| m.id == ids[0]));

    let b = Pc::new(Arc::new(FakeSemantic { model: "it-embed-b" }));
    b.pair(&cloud, "pc-model-b").await;
    b.sync().await;
    assert_eq!(b.memory.embedding_status().expect("estado").pending, 0, "los vectores llegaron del Cloud");
}

/// F: un device revocado deja de sincronizar (y Jarvis sigue funcionando en local).
#[tokio::test]
async fn scenario_f_a_revoked_device_cannot_sync() {
    let cloud = require_cloud!();
    let pc = Pc::new(Arc::new(HashingEmbedder::default()));
    pc.pair(&cloud, "pc-revocada").await;
    pc.remember("Antes de revocar").await;
    pc.sync().await;
    let device = pc.sync.status().device_id.expect("device");

    let token = owner_token(&cloud).await;
    let revoked = reqwest::Client::new()
        .post(format!("{}/api/v1/devices/{device}/revoke", cloud.url))
        .bearer_auth(token)
        .json(&serde_json::json!({ "reason": "equipo perdido" }))
        .send()
        .await
        .expect("revocar");
    assert!(revoked.status().is_success());

    pc.remember("Después de revocar").await;
    let reopened = pc.reopen();
    let error = reopened.run_once().await.expect_err("revocado");
    assert_eq!(error, CloudError::Unauthorized);
    assert!(reopened.status().pending >= 1, "el cambio local se conserva");
    assert!(!pc.memory.retrieve("después de revocar", &RetrievalOptions::default()).await.expect("local").items.is_empty());
}

/// G: restaurar la memoria desde un respaldo validado en una PC nueva y volver a sincronizar sin duplicados.
#[tokio::test]
async fn scenario_g_restore_from_a_validated_backup() {
    let cloud = require_cloud!();
    let a = Pc::new(Arc::new(HashingEmbedder::default()));
    a.pair(&cloud, "pc-respaldo").await;
    let id = a.remember("El router de casa es un TP-Link en la sala").await;
    a.sync().await;
    let package = a._dir.path().join("respaldo.jarvismem");
    let kdf = jarvis_desktop_lib::memory::memory_package::KdfParams { m_cost_kib: 8 * 1024, t_cost: 1, p_cost: 1 };
    a.memory.export_memory(&package, "frase-de-respaldo-larga", &Default::default(), kdf).await.expect("exportar");

    let c = Pc::new(Arc::new(HashingEmbedder::default()));
    assert!(c.memory.preview_import(&package, "otra-frase-equivocada").is_err(), "sin la frase correcta no se abre");
    let preview = c.memory.preview_import(&package, "frase-de-respaldo-larga").expect("vista previa válida");
    assert!(!preview.memories.is_empty());
    c.memory.apply_import(&package, "frase-de-respaldo-larga", &Default::default()).await.expect("importar");
    assert!(c.content(&id).is_some());

    c.pair(&cloud, "pc-restaurada").await;
    let report = c.sync().await;
    assert_eq!(report.conflicts, 0, "{report:?}");
    assert_eq!(report.rejected, 0, "{report:?}");
    assert_eq!(c.sync.status().pending, 0);
    assert_eq!(c.sync.status().mode, SyncMode::Hybrid);
}

/// H: la credencial del equipo emparejado abre los servicios del servidor (lo que nginx comprueba con
/// `auth_request` para /ollama, /stt y /tts); un equipo sin emparejar no tiene credencial.
#[tokio::test]
async fn scenario_h_the_device_credential_opens_the_server_services() {
    use jarvis_desktop_lib::infrastructure::llm::BearerSource;
    let cloud = require_cloud!();
    let pc = Pc::new(Arc::new(HashingEmbedder::default()));
    assert!(pc.sync.bearer().await.is_none(), "sin emparejar no hay credencial");
    pc.pair(&cloud, "pc-servicios").await;
    assert_eq!(pc.sync.server_url().as_deref(), Some(cloud.url.trim_end_matches('/')));
    let token = pc.sync.bearer().await.expect("token del device");
    let check = reqwest::Client::new()
        .get(format!("{}/api/v1/access/check", cloud.url))
        .bearer_auth(&token)
        .send()
        .await
        .expect("check");
    assert_eq!(check.status().as_u16(), 204);
    let anonymous = reqwest::Client::new().get(format!("{}/api/v1/access/check", cloud.url)).send().await.expect("check");
    assert_eq!(anonymous.status().as_u16(), 401);
}
