//! Recarga de `policy.toml` sin reiniciar (HU-WIN-024).
//!
//! Regla del diseño: una política inválida **no** sustituye a la anterior. Si nunca hubo una válida,
//! el agente ya arrancó en modo fail-closed (solo R0).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use jarvis_policy::{Policy, PolicyState};
use sha2::{Digest, Sha256};

use crate::health::{Health, Severity, codes};
use crate::pipeline::Agent;
use crate::tools::ToolContext;

pub struct PolicyReloader {
    path: PathBuf,
    agent: Arc<Agent>,
    tools: Arc<ToolContext>,
    health: Arc<Health>,
    state: Mutex<Watched>,
}

#[derive(Debug, Default)]
struct Watched {
    modified: Option<SystemTime>,
    hash: String,
}

fn hash(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

impl PolicyReloader {
    #[must_use]
    pub fn new(path: PathBuf, agent: Arc<Agent>, tools: Arc<ToolContext>, health: Arc<Health>) -> Arc<Self> {
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let hash_now = std::fs::read_to_string(&path).map(|text| hash(&text)).unwrap_or_default();
        Arc::new(Self { path, agent, tools, health, state: Mutex::new(Watched { modified, hash: hash_now }) })
    }

    /// Relee el archivo y sustituye la política si es válida.
    ///
    /// # Errors
    /// Mensaje del error de validación; la política anterior se conserva.
    pub fn reload(&self) -> Result<String, String> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let policy = Policy::load(&self.path).map_err(|e| e.to_string())?;
        let new_hash = hash(&text);
        let previous = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::mem::replace(&mut state.hash, new_hash.clone());
            state.modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
            previous
        };

        let policy = Arc::new(policy);
        self.agent.set_policy(Arc::new(PolicyState { policy: (*policy).clone(), valid: true }));
        self.tools.set_policy(policy.clone());
        self.health.set_policy(true);
        self.health.clear(codes::POLICY_INVALID);
        // Auditoría: queda constancia de cada cambio de la frontera de seguridad.
        tracing::info!(previous = %previous, current = %new_hash, "local policy reloaded");
        Ok(format!("policy reloaded: {} principals, {} deny rules", policy.principals.len(), policy.deny.len()))
    }

    /// Recarga solo si el archivo cambió desde la última vez (lo llama el bucle de mantenimiento).
    pub fn reload_if_changed(&self) {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        let unchanged = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.modified == modified
        };
        if unchanged {
            return;
        }
        match self.reload() {
            Ok(message) => tracing::info!(%message, "policy.toml changed on disk"),
            Err(error) => {
                tracing::error!(%error, "policy.toml changed but is invalid: keeping the previous one");
                self.health.raise(codes::POLICY_INVALID, Severity::Error, error);
                // Se recuerda la marca de tiempo para no repetir el aviso en cada vuelta.
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.modified = modified;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_tooling::{Executor, ExecutorConfig};

    /// Doble mínimo de plataforma: la recarga no toca Windows.
    struct NoWindows;

    impl jarvis_tooling::platform::WindowsApi for NoWindows {
        fn processes(
            &self,
        ) -> Result<Vec<jarvis_tooling::platform::ProcessInfo>, jarvis_tooling::platform::PlatformError> {
            Ok(Vec::new())
        }
        fn top_level_windows(
            &self,
        ) -> Result<Vec<jarvis_tooling::platform::WindowInfo>, jarvis_tooling::platform::PlatformError> {
            Ok(Vec::new())
        }
        fn foreground_window(
            &self,
            _include_title: bool,
        ) -> Result<Option<jarvis_tooling::platform::ForegroundWindow>, jarvis_tooling::platform::PlatformError>
        {
            Ok(None)
        }
        fn launch(
            &self,
            _target: &jarvis_tooling::platform::LaunchTarget,
        ) -> Result<jarvis_tooling::platform::Launch, jarvis_tooling::platform::PlatformError> {
            unreachable!("no se lanza nada en este test")
        }
        fn request_close(&self, _pids: &[u32]) -> Result<usize, jarvis_tooling::platform::PlatformError> {
            Ok(0)
        }
        fn terminate(
            &self,
            _pid: u32,
            _created_at: Option<SystemTime>,
        ) -> Result<(), jarvis_tooling::platform::PlatformError> {
            Ok(())
        }
        fn session_state(&self) -> jarvis_tooling::platform::SessionState {
            jarvis_tooling::platform::SessionState::Active
        }
        fn resolve_app_path(&self, _exe_name: &str) -> Option<PathBuf> {
            None
        }
        fn expand_env(&self, text: &str) -> String {
            text.to_owned()
        }
        fn is_trusted_executable(&self, _path: &std::path::Path) -> bool {
            true
        }
        fn open_sandboxed(
            &self,
            _path: &std::path::Path,
        ) -> Result<jarvis_tooling::platform::Opened, jarvis_tooling::platform::PlatformError> {
            unreachable!("no se abre nada en este test")
        }
        fn run_command(
            &self,
            _spec: &jarvis_tooling::platform::CommandSpec,
            _cancel: &jarvis_tooling::platform::CancelFlag,
        ) -> Result<jarvis_tooling::platform::CommandOutput, jarvis_tooling::platform::PlatformError> {
            unreachable!("no se ejecuta nada en este test")
        }
        fn focus_window(&self, _pids: &[u32]) -> Result<bool, jarvis_tooling::platform::PlatformError> {
            Ok(false)
        }
        fn foreground_pid(&self) -> Result<Option<u32>, jarvis_tooling::platform::PlatformError> {
            Ok(None)
        }
        fn show_notification(&self, _title: &str, _body: &str) -> Result<(), jarvis_tooling::platform::PlatformError> {
            Ok(())
        }
        fn ui_window(
            &self,
            _hwnd: isize,
            _timeout: std::time::Duration,
        ) -> Result<jarvis_tooling::platform::UiWindow, jarvis_tooling::platform::PlatformError> {
            unreachable!("no se usa UI Automation en este test")
        }
        fn ui_find(
            &self,
            _hwnd: isize,
            _selector: &jarvis_tooling::platform::UiSelector,
            _limit: usize,
            _timeout: std::time::Duration,
        ) -> Result<Vec<jarvis_tooling::platform::UiElement>, jarvis_tooling::platform::PlatformError> {
            Ok(Vec::new())
        }
        fn ui_invoke(
            &self,
            _hwnd: isize,
            _selector: &jarvis_tooling::platform::UiSelector,
            _timeout: std::time::Duration,
        ) -> Result<String, jarvis_tooling::platform::PlatformError> {
            unreachable!("no se actúa sobre la UI en este test")
        }
        fn ui_set_value(
            &self,
            _hwnd: isize,
            _selector: &jarvis_tooling::platform::UiSelector,
            _text: &str,
            _timeout: std::time::Duration,
        ) -> Result<(), jarvis_tooling::platform::PlatformError> {
            unreachable!("no se escribe en la UI en este test")
        }
    }

    struct Silent;

    #[async_trait::async_trait]
    impl crate::pipeline::Publisher for Silent {
        async fn publish(&self, _channel: jarvis_protocol::Channel, _payload: Vec<u8>) -> bool {
            true
        }
    }

    fn harness(policy_text: &str) -> (tempfile::TempDir, Arc<PolicyReloader>, Arc<Agent>, Arc<ToolContext>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, policy_text).expect("write");
        let policy = Arc::new(Policy::load(&path).expect("valid"));
        let tools = ToolContext::new(Arc::new(NoWindows), policy.clone());
        let registry =
            Arc::new(crate::tools::build_registry(&tools, &["is_app_running".to_owned()]).expect("registry"));
        let store = Arc::new(jarvis_state::Store::open(&dir.path().join("agent.db")).expect("store"));
        let health = Arc::new(Health::new());
        let agent = Agent::new(
            uuid::Uuid::nil(),
            store,
            registry,
            Arc::new(Executor::new(ExecutorConfig::default())),
            Arc::new(PolicyState { policy: (*policy).clone(), valid: true }),
            health.clone(),
            Arc::new(Silent),
        );
        let reloader = PolicyReloader::new(path, agent.clone(), tools.clone(), health);
        (dir, reloader, agent, tools)
    }

    const BASE: &str = "version = 1\n[[principals]]\nid = \"local:owner\"\ncapabilities = [\"windows.apps.read\"]\n";

    #[test]
    fn a_valid_change_replaces_the_policy_everywhere() {
        let (dir, reloader, agent, tools) = harness(BASE);
        assert_eq!(agent.policy().policy.principals.len(), 1);
        assert!(tools.policy().apps.get("vscode").is_none());

        let updated = format!(
            "{BASE}\n[apps.vscode]\ndisplay_name = \"Visual Studio Code\"\nlaunch = {{ kind = \"app_paths\", name = \"Code.exe\" }}\nprocess_names = [\"Code.exe\"]\n"
        );
        std::fs::write(dir.path().join("policy.toml"), updated).expect("write");
        reloader.reload().expect("reload");

        assert!(tools.policy().apps.get("vscode").is_some(), "el catálogo de las tools también se actualiza");
        assert!(agent.policy().valid);
    }

    #[test]
    fn an_invalid_change_keeps_the_previous_policy() {
        let (dir, reloader, agent, _tools) = harness(BASE);
        std::fs::write(dir.path().join("policy.toml"), "version = 1\n[[principals]]\nid = \"cloud:nope\"\n")
            .expect("write");

        let error = reloader.reload().expect_err("debe fallar");
        assert!(error.contains("unknown principal"), "{error}");
        assert_eq!(agent.policy().policy.principals[0].id, "local:owner");
        assert!(agent.policy().valid, "la política anterior sigue siendo válida");
    }

    #[test]
    fn reload_if_changed_only_acts_on_real_changes() {
        let (dir, reloader, agent, tools) = harness(BASE);
        reloader.reload_if_changed();
        assert!(tools.policy().apps.get("vscode").is_none());

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let updated = format!(
            "{BASE}\n[apps.vscode]\ndisplay_name = \"Code\"\nlaunch = {{ kind = \"path\", path = \"C:/Code.exe\" }}\nprocess_names = [\"Code.exe\"]\n"
        );
        std::fs::write(dir.path().join("policy.toml"), updated).expect("write");
        reloader.reload_if_changed();
        assert!(tools.policy().apps.get("vscode").is_some());
        assert!(agent.policy().valid);
    }
}
