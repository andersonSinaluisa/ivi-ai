//! Configuración: valores por defecto incrustados (`config/jarvis.toml`), el archivo del repo si se
//! encuentra, y los cambios del usuario en `%LOCALAPPDATA%\Jarvis\settings.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::policies::ToolMode;
use crate::domain::RiskClass;

const DEFAULT_TOML: &str = include_str!("../../../../config/jarvis.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct LlmSection {
    pub provider: String,
    pub model: String,
    pub temperature: f32,
    pub num_ctx: u32,
    pub think: bool,
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentSection {
    pub max_steps: usize,
    pub history_messages: usize,
    pub approval_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicySection {
    pub auto_approve: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowsAgentSection {
    #[serde(default)]
    pub pipe: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceSection {
    pub base_url: String,
}

/// Servidor (Cloud Core) que se propone al emparejar. Modelos, voz y memoria van por él.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CloudSection {
    #[serde(default)]
    pub default_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemorySection {
    /// Modelo de embeddings en el Ollama del servidor. Si no está, se usa el respaldo léxico.
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Recuerdos que se envían al modelo por instrucción.
    #[serde(default = "default_memories_per_turn")]
    pub memories_per_turn: usize,
}

fn default_embedding_model() -> String {
    "qwen3-embedding:0.6b".into()
}

fn default_memories_per_turn() -> usize {
    6
}

impl Default for MemorySection {
    fn default() -> Self {
        Self { embedding_model: default_embedding_model(), memories_per_turn: default_memories_per_turn() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileConfig {
    pub llm: LlmSection,
    pub agent: AgentSection,
    pub policy: PolicySection,
    pub windows_agent: WindowsAgentSection,
    pub voice: VoiceSection,
    #[serde(default)]
    pub memory: MemorySection,
    #[serde(default)]
    pub cloud: CloudSection,
}

/// Lo que el usuario cambia desde la UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettings {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub num_ctx: Option<u32>,
    pub think: Option<bool>,
    pub max_steps: Option<usize>,
    pub auto_approve: Option<Vec<RiskClass>>,
    #[serde(default)]
    pub tool_modes: BTreeMap<String, ToolMode>,
    /// Inference Gateway: dónde corre el modelo (la clave, si hay, va al almacén de secretos).
    #[serde(default)]
    pub inference: InferenceSettings,
}

/// Proveedor de inferencia (Configuración → Modelos).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceSettings {
    pub mode: crate::infrastructure::llm::gateway::InferenceMode,
    /// `ollama` (Ollama remoto) u `openai` (API compatible con chat completions).
    pub remote_kind: String,
    pub remote_url: String,
    pub remote_model: String,
}

impl Default for InferenceSettings {
    fn default() -> Self {
        Self {
            mode: crate::infrastructure::llm::gateway::InferenceMode::Server,
            remote_kind: "ollama".into(),
            remote_url: String::new(),
            remote_model: String::new(),
        }
    }
}

impl InferenceSettings {
    /// # Errors
    /// Datos incompletos o URL insegura hacia un host remoto.
    pub fn validate(&self) -> Result<(), String> {
        if self.remote_kind != "ollama" && self.remote_kind != "openai" {
            return Err("el proveedor remoto debe ser 'ollama' u 'openai'".into());
        }
        if self.remote_url.trim().is_empty() {
            return if self.mode == crate::infrastructure::llm::gateway::InferenceMode::Server {
                Ok(())
            } else {
                Err("falta la URL del proveedor remoto".into())
            };
        }
        let url = reqwest::Url::parse(self.remote_url.trim()).map_err(|e| format!("URL inválida: {e}"))?;
        let host = url.host_str().unwrap_or_default();
        let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]");
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err("un proveedor remoto requiere HTTPS (HTTP solo hacia localhost)".into());
        }
        if self.remote_model.trim().is_empty() {
            return Err("falta el modelo del proveedor remoto".into());
        }
        Ok(())
    }
}

/// Vista combinada que se muestra en Configuración.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub model: String,
    pub temperature: f32,
    pub num_ctx: u32,
    pub think: bool,
    pub max_steps: usize,
    pub auto_approve: Vec<RiskClass>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub file: FileConfig,
    pub user: UserSettings,
    pub config_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
}

impl AppConfig {
    /// # Errors
    /// Si la configuración incrustada o la del repo no son válidas.
    pub fn load() -> Result<Self, String> {
        let config_dir = find_config_dir();
        let text = config_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("jarvis.toml")).ok())
            .unwrap_or_else(|| DEFAULT_TOML.to_owned());
        let file: FileConfig = toml::from_str(&text).map_err(|e| format!("jarvis.toml inválido: {e}"))?;
        let data_dir = data_dir();
        let user = std::fs::read_to_string(data_dir.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Ok(Self { file, user, config_dir, data_dir })
    }

    #[must_use]
    pub fn settings(&self) -> Settings {
        let f = &self.file;
        Settings {
            model: self.user.model.clone().unwrap_or_else(|| f.llm.model.clone()),
            temperature: self.user.temperature.unwrap_or(f.llm.temperature),
            num_ctx: self.user.num_ctx.unwrap_or(f.llm.num_ctx),
            think: self.user.think.unwrap_or(f.llm.think),
            max_steps: self.user.max_steps.unwrap_or(f.agent.max_steps),
            auto_approve: self
                .user
                .auto_approve
                .clone()
                .unwrap_or_else(|| f.policy.auto_approve.iter().filter_map(|s| RiskClass::parse(s)).collect()),
        }
    }

    /// Aplica y guarda los cambios de la UI.
    ///
    /// # Errors
    /// Si no se puede escribir el archivo.
    pub fn save_settings(&mut self, s: &Settings) -> Result<(), String> {
        if s.model.trim().is_empty() {
            return Err("falta el modelo".into());
        }
        self.user.model = Some(s.model.trim().to_owned());
        self.user.temperature = Some(s.temperature.clamp(0.0, 2.0));
        self.user.num_ctx = Some(s.num_ctx.clamp(1024, 262_144));
        self.user.think = Some(s.think);
        self.user.max_steps = Some(s.max_steps.clamp(1, 20));
        self.user.auto_approve = Some(s.auto_approve.iter().copied().filter(|r| *r != RiskClass::Destructive).collect());
        self.persist()
    }

    /// # Errors
    /// Si no se puede escribir el archivo.
    pub fn persist(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&self.user).map_err(|e| e.to_string())?;
        std::fs::write(self.data_dir.join("settings.json"), text).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn personality_yaml(&self) -> Option<String> {
        self.config_dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("personality.yaml")).ok())
    }
}

/// `%LOCALAPPDATA%\Jarvis` en Windows, `~/.local/share/Jarvis` en Linux.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("JARVIS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("Jarvis")
}

fn has_config(dir: &Path) -> bool {
    dir.join("config").join("jarvis.toml").is_file()
}

/// Busca la carpeta `config/` del monorepo: `JARVIS_CONFIG_DIR`, o subiendo desde el directorio
/// actual o desde el ejecutable.
#[must_use]
pub fn find_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("JARVIS_CONFIG_DIR") {
        let p = PathBuf::from(dir);
        if p.join("jarvis.toml").is_file() {
            return Some(p);
        }
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))];
    for start in starts.into_iter().flatten() {
        for dir in start.ancestors() {
            if has_config(dir) {
                return Some(dir.join("config"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_config_parses_with_safe_defaults() {
        let file: FileConfig = toml::from_str(DEFAULT_TOML).expect("toml");
        assert_eq!(file.llm.provider, "ollama");
        assert!(file.cloud.default_url.starts_with("https://"));
        assert!(file.voice.base_url.starts_with("http://127.0.0.1"));
        assert!(!file.policy.auto_approve.iter().any(|s| s == "DESTRUCTIVE"));
    }

    #[test]
    fn user_settings_override_and_destructive_cannot_be_auto_approved() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut cfg = AppConfig {
            file: toml::from_str(DEFAULT_TOML).expect("toml"),
            user: UserSettings::default(),
            config_dir: None,
            data_dir: dir.path().to_path_buf(),
        };
        let mut s = cfg.settings();
        s.model = "qwen3:8b".into();
        s.auto_approve = vec![RiskClass::ReadOnly, RiskClass::Destructive];
        cfg.save_settings(&s).expect("save");
        let s2 = cfg.settings();
        assert_eq!(s2.model, "qwen3:8b");
        assert_eq!(s2.auto_approve, vec![RiskClass::ReadOnly]);
        assert!(dir.path().join("settings.json").is_file());

        s.model = "  ".into();
        assert!(cfg.save_settings(&s).is_err());
    }
}
