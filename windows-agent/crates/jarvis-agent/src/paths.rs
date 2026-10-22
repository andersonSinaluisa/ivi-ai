//! Rutas del agente (docs/01-architecture/windows-agent-architecture.md#directorios).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// `%LOCALAPPDATA%\Jarvis\Agent`, o la raíz indicada (tests y desarrollo).
    ///
    /// # Errors
    /// Si Windows no devuelve la carpeta conocida.
    pub fn resolve(override_root: Option<PathBuf>) -> anyhow::Result<Self> {
        let root = match override_root {
            Some(root) => root,
            None => local_app_data()?.join("Jarvis").join("Agent"),
        };
        Ok(Self { root })
    }

    #[must_use]
    pub fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    #[must_use]
    pub fn policy(&self) -> PathBuf {
        self.root.join("policy.toml")
    }

    #[must_use]
    pub fn database(&self) -> PathBuf {
        self.root.join("state").join("agent.db")
    }

    #[must_use]
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// # Errors
    /// Si no se pueden crear los directorios.
    pub fn ensure(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.root.join("state"))?;
        std::fs::create_dir_all(self.logs())?;
        Ok(())
    }
}

#[cfg(windows)]
fn local_app_data() -> anyhow::Result<PathBuf> {
    jarvis_windows::local_app_data().map_err(|e| anyhow::anyhow!("cannot resolve %LOCALAPPDATA%: {e}"))
}

#[cfg(not(windows))]
fn local_app_data() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share"))
}

/// Nombre del named pipe de la CLI (ACL restringida al SID del usuario).
#[must_use]
pub fn pipe_name(user_sid: &str) -> String {
    format!(r"\\.\pipe\jarvis-agent-{user_sid}")
}
