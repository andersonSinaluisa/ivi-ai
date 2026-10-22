//! Dónde se guarda el secreto del device (credencial del Cloud Core) y las claves de proveedores de IA.
//!
//! - Windows: **Credential Manager** (como el windows-agent), vía `keyring`.
//! - Otros sistemas (desarrollo y pruebas): archivo con permisos 0600 en la carpeta de datos.
//!
//! Nunca en la base de datos, en la configuración ni en los logs.

use std::path::PathBuf;
use std::sync::Mutex;

pub trait SecretVault: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    /// # Errors
    /// Si el almacén del sistema rechaza la operación.
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    /// # Errors
    /// Si el almacén del sistema rechaza la operación.
    fn delete(&self, key: &str) -> Result<(), String>;
    fn kind(&self) -> &'static str;
}

pub const SERVICE: &str = "Jarvis.Desktop";

#[cfg(windows)]
pub struct CredentialManagerVault;

#[cfg(windows)]
impl SecretVault for CredentialManagerVault {
    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value)).map_err(|e| e.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn kind(&self) -> &'static str {
        "Windows Credential Manager"
    }
}

/// Archivo por clave con permisos de solo el usuario (0600 en Unix).
pub struct FileVault {
    dir: PathBuf,
}

impl FileVault {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, key: &str) -> PathBuf {
        let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        self.dir.join(format!("{safe}.secret"))
    }
}

impl SecretVault for FileVault {
    fn get(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.path(key)).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.path(key);
        std::fs::write(&path, value).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn kind(&self) -> &'static str {
        "archivo protegido (solo desarrollo)"
    }
}

/// En memoria (pruebas).
#[derive(Default)]
pub struct MemoryVault {
    values: Mutex<std::collections::HashMap<String, String>>,
}

impl SecretVault for MemoryVault {
    fn get(&self, key: &str) -> Option<String> {
        self.values.lock().ok()?.get(key).cloned()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.values.lock().map_err(|e| e.to_string())?.insert(key.into(), value.into());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.values.lock().map_err(|e| e.to_string())?.remove(key);
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "memoria (pruebas)"
    }
}

/// Almacén por defecto de la plataforma.
#[must_use]
pub fn platform_vault(data_dir: &std::path::Path) -> std::sync::Arc<dyn SecretVault> {
    #[cfg(windows)]
    {
        let _ = data_dir;
        std::sync::Arc::new(CredentialManagerVault)
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(FileVault::new(data_dir.join("secrets")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_vault_round_trip_with_private_permissions() {
        let dir = tempfile::tempdir().expect("tmp");
        let vault = FileVault::new(dir.path().join("secrets"));
        assert_eq!(vault.get("cloud/device"), None);
        vault.set("cloud/device", "jdv_secreto").expect("set");
        assert_eq!(vault.get("cloud/device").as_deref(), Some("jdv_secreto"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secrets/cloud_device.secret")).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        vault.delete("cloud/device").expect("delete");
        vault.delete("cloud/device").expect("idempotente");
        assert_eq!(vault.get("cloud/device"), None);
    }
}
