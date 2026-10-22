//! Estado compartido de la aplicación y su construcción.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::core::agent::orchestrator::approval_timeout;
use crate::core::agent::{AgentConfig, AgentDeps, AgentOrchestrator, ApprovalGate, DefaultObserver, Personality};
use crate::core::executor::DefaultExecutor;
use crate::core::planner::LlmPlanner;
use crate::core::policies::{PolicyConfig, PolicyEngine};
use crate::core::verifier::DefaultVerifier;
use crate::domain::{RiskClass, ToolSource};
use crate::infrastructure::llm::gateway::{InferenceGateway, InferenceMode};
use crate::infrastructure::llm::ollama::{OllamaOptions, OllamaProvider};
use crate::infrastructure::llm::openai_compat::{OpenAiCompatOptions, OpenAiCompatProvider};
use crate::infrastructure::llm::LlmProvider;
use crate::sync::retrieval::HybridRetriever;
use crate::sync::secrets::{platform_vault, SecretVault};
use crate::sync::SyncEngine;
use crate::memory::embedding_provider::HashingEmbedder;
use crate::memory::episodic_memory::EpisodicStore;
use crate::memory::{MemoryConfig, MemoryEngine};
use crate::infrastructure::persistence::Database;
use crate::infrastructure::windows::tool_api::{default_pipe_name, NamedPipeConnector};
use crate::infrastructure::windows::ToolApiClient;
use crate::tools::local::{GetCurrentTime, GetSystemInfo, SaveMemory, SearchMemory};
use crate::tools::remote::RemoteTool;
use crate::tools::{Tool, ToolRegistry};

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLinkStatus {
    pub ok: bool,
    pub detail: String,
    pub tool_count: usize,
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub db: Database,
    pub memory: Arc<MemoryEngine>,
    /// Trabajo largo de memoria en curso (recalcular embeddings), cancelable.
    pub memory_job: Mutex<Option<CancellationToken>>,
    /// Ollama local (también lo usan la lista de modelos y el estado).
    pub provider: Arc<OllamaProvider>,
    /// Inference Gateway: lo que usa el agente (local, remoto o automático).
    pub gateway: Arc<InferenceGateway>,
    /// Sincronización con el Cloud Core (local primero).
    pub sync: Arc<SyncEngine>,
    /// Credencial del device y claves de proveedores: Credential Manager en Windows.
    pub vault: Arc<dyn SecretVault>,
    pub registry: Arc<ToolRegistry>,
    pub policy: Arc<RwLock<PolicyEngine>>,
    pub agent_config: Arc<RwLock<AgentConfig>>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub runs: Mutex<HashMap<String, CancellationToken>>,
    pub agent_link: RwLock<AgentLinkStatus>,
    pub tool_api: tokio::sync::Mutex<Option<Arc<ToolApiClient>>>,
}

fn ollama_options(cfg: &AppConfig) -> OllamaOptions {
    let s = cfg.settings();
    OllamaOptions {
        base_url: s.base_url,
        model: s.model,
        temperature: s.temperature,
        num_ctx: s.num_ctx,
        think: s.think,
        request_timeout: Duration::from_secs(cfg.file.llm.request_timeout_secs.max(10)),
        api_key: None,
    }
}

pub const INFERENCE_KEY: &str = "inference-api-key";

/// Configura el Inference Gateway con los ajustes del usuario (la clave sale del almacén de secretos).
pub fn configure_gateway(gateway: &InferenceGateway, cfg: &AppConfig, vault: &dyn SecretVault) {
    let inference = &cfg.user.inference;
    let remote: Option<Arc<dyn LlmProvider>> = if inference.validate().is_err() || inference.remote_url.trim().is_empty() {
        None
    } else {
        let key = vault.get(INFERENCE_KEY);
        let timeout = Duration::from_secs(cfg.file.llm.request_timeout_secs.max(10));
        let s = cfg.settings();
        if inference.remote_kind == "openai" {
            OpenAiCompatProvider::new(OpenAiCompatOptions {
                base_url: inference.remote_url.trim().to_owned(),
                model: inference.remote_model.trim().to_owned(),
                api_key: key,
                temperature: s.temperature,
                request_timeout: timeout,
            })
            .ok()
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        } else {
            OllamaProvider::new(OllamaOptions {
                base_url: inference.remote_url.trim().to_owned(),
                model: inference.remote_model.trim().to_owned(),
                temperature: s.temperature,
                num_ctx: s.num_ctx,
                think: s.think,
                request_timeout: timeout,
                api_key: key,
            })
            .ok()
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        }
    };
    let mode = if remote.is_none() && inference.mode != InferenceMode::Local { InferenceMode::Local } else { inference.mode };
    gateway.configure(mode, remote);
}

pub fn policy_config(cfg: &AppConfig) -> PolicyConfig {
    let s = cfg.settings();
    PolicyConfig {
        auto_approve: s.auto_approve.into_iter().collect::<BTreeSet<RiskClass>>(),
        deny: cfg.file.policy.deny.iter().cloned().collect(),
        tool_modes: cfg.user.tool_modes.clone(),
    }
}

impl AppState {
    /// # Errors
    /// Si no se puede abrir la base de datos o la configuración es inválida.
    pub fn build(config: AppConfig) -> Result<Self, String> {
        let db = Database::open(&config.data_dir.join("jarvis.db")).map_err(|e| e.to_string())?;
        if let Ok(n) = db.recover_interrupted_runs() {
            if n > 0 {
                tracing::info!(n, "ejecuciones interrumpidas marcadas como fallidas");
            }
        }
        // Respaldo léxico hasta comprobar el modelo de embeddings (en segundo plano, ver `select_embedder`).
        let memory = Arc::new(
            MemoryEngine::new(db.clone(), Arc::new(HashingEmbedder::default()), MemoryConfig::in_dir(&config.data_dir))
                .map_err(|e| e.to_string())?,
        );
        match memory.consolidate() {
            Ok(r) => tracing::info!(?r, "consolidación de memoria"),
            Err(e) => tracing::warn!(error = %e, "consolidación fallida"),
        }

        let provider = Arc::new(OllamaProvider::new(ollama_options(&config)).map_err(|e| e.to_string())?);
        let gateway = Arc::new(InferenceGateway::new(provider.clone()));
        let vault = platform_vault(&config.data_dir);
        configure_gateway(&gateway, &config, vault.as_ref());
        let sync = SyncEngine::new(memory.clone(), vault.clone());
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(GetCurrentTime::new()))?;
        registry.register(Arc::new(GetSystemInfo::new()))?;
        registry.register(Arc::new(SaveMemory::new(memory.clone())))?;
        registry.register(Arc::new(SearchMemory::new(memory.clone())))?;

        let policy = Arc::new(RwLock::new(PolicyEngine::new(policy_config(&config))));
        let personality = config
            .personality_yaml()
            .and_then(|y| Personality::from_yaml(&y).map_err(|e| tracing::warn!(%e, "uso la personalidad por defecto")).ok())
            .unwrap_or_else(Personality::default_personality);
        let agent_config = Arc::new(RwLock::new(AgentConfig {
            max_steps: config.settings().max_steps,
            history_messages: config.file.agent.history_messages,
            memories_per_turn: config.file.memory.memories_per_turn,
        }));

        let orchestrator = Arc::new(AgentOrchestrator::new(AgentDeps {
            observer: Arc::new(DefaultObserver::new(registry.clone())),
            planner: Arc::new(LlmPlanner::new(gateway.clone())),
            registry: registry.clone(),
            policy: policy.clone(),
            approvals: Arc::new(ApprovalGate::new(approval_timeout(config.file.agent.approval_timeout_secs))),
            executor: Arc::new(DefaultExecutor),
            verifier: Arc::new(DefaultVerifier),
            episodic: Arc::new(EpisodicStore::new(db.clone())),
            retriever: Arc::new(HybridRetriever::new(memory.clone(), sync.clone())),
            procedural: memory.clone(),
            db: db.clone(),
            personality: Arc::new(RwLock::new(personality)),
            config: agent_config.clone(),
            visual: Some({
                let m = memory.clone();
                Arc::new(move |e| m.emit(e))
            }),
        }));

        Ok(Self {
            config: Mutex::new(config),
            db,
            memory,
            memory_job: Mutex::new(None),
            provider,
            gateway,
            sync,
            vault,
            registry,
            policy,
            agent_config,
            orchestrator,
            runs: Mutex::new(HashMap::new()),
            agent_link: RwLock::new(AgentLinkStatus { ok: false, detail: "sin comprobar".into(), tool_count: 0 }),
            tool_api: tokio::sync::Mutex::new(None),
        })
    }

    pub fn apply_settings(&self) {
        let Ok(cfg) = self.config.lock() else { return };
        self.provider.set_options(ollama_options(&cfg));
        configure_gateway(&self.gateway, &cfg, self.vault.as_ref());
        if let Ok(mut p) = self.policy.write() {
            *p = PolicyEngine::new(policy_config(&cfg));
        }
        if let Ok(mut a) = self.agent_config.write() {
            a.max_steps = cfg.settings().max_steps;
        }
    }

    /// Usa el modelo de embeddings de Ollama si responde; si no, el respaldo léxico. Si el modelo
    /// cambió, recalcula los vectores que falten.
    pub async fn select_embedder(&self) -> String {
        let (base_url, model) = match self.config.lock() {
            Ok(c) => (c.settings().base_url, c.file.memory.embedding_model.clone()),
            Err(_) => return "configuración no disponible".into(),
        };
        let (embedder, detail) = MemoryEngine::select_embedder(&base_url, &model).await;
        self.memory.set_embedder(embedder, detail.clone());
        let cancel = CancellationToken::new();
        if let Ok(mut job) = self.memory_job.lock() {
            *job = Some(cancel.clone());
        }
        match self.memory.reembed(&cancel).await {
            Ok(n) if n > 0 => tracing::info!(n, "vectores recalculados"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "no se pudieron recalcular los vectores"),
        }
        if let Ok(mut job) = self.memory_job.lock() {
            *job = None;
        }
        detail
    }

    /// (Re)conecta con el windows-agent y refresca sus herramientas. Nunca bloquea el arranque:
    /// si falla, Jarvis sigue con las herramientas locales.
    pub async fn refresh_windows_agent(&self) -> AgentLinkStatus {
        let (enabled, pipe) = match self.config.lock() {
            Ok(c) => (c.file.windows_agent.enabled, c.file.windows_agent.pipe.clone()),
            Err(_) => (false, String::new()),
        };
        let status = if enabled { self.connect_agent(pipe).await } else {
            self.registry.replace_source(ToolSource::WindowsAgent, Vec::new());
            AgentLinkStatus { ok: false, detail: "desactivado en jarvis.toml".into(), tool_count: 0 }
        };
        if let Ok(mut s) = self.agent_link.write() {
            *s = status.clone();
        }
        status
    }

    async fn connect_agent(&self, pipe: String) -> AgentLinkStatus {
        let pipe = if pipe.trim().is_empty() {
            match default_pipe_name().await {
                Ok(p) => p,
                Err(e) => {
                    self.registry.replace_source(ToolSource::WindowsAgent, Vec::new());
                    return AgentLinkStatus { ok: false, detail: e.to_string(), tool_count: 0 };
                }
            }
        } else {
            pipe
        };
        let client = Arc::new(ToolApiClient::new(Arc::new(NamedPipeConnector { pipe }), Duration::from_secs(70)));
        match client.list_tools().await {
            Ok(descriptors) => {
                let tools: Vec<Arc<dyn Tool>> =
                    descriptors.into_iter().map(|d| Arc::new(RemoteTool::new(d, client.clone())) as Arc<dyn Tool>).collect();
                let rejected = self.registry.replace_source(ToolSource::WindowsAgent, tools);
                for r in &rejected {
                    tracing::warn!(%r, "herramienta remota rechazada");
                }
                *self.tool_api.lock().await = Some(client);
                let count = self.registry.count(ToolSource::WindowsAgent);
                AgentLinkStatus { ok: true, detail: format!("conectado ({count} herramientas)"), tool_count: count }
            }
            Err(e) => {
                self.registry.replace_source(ToolSource::WindowsAgent, Vec::new());
                *self.tool_api.lock().await = None;
                AgentLinkStatus { ok: false, detail: e.to_string(), tool_count: 0 }
            }
        }
    }
}
