//! Credenciales en Windows Credential Manager (ADR-009).
//!
//! El secreto del device **nunca** se guarda en archivos de configuración, variables de entorno ni
//! en la base local: solo aquí, protegido por DPAPI del usuario.

use std::fmt;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use zeroize::Zeroize;

/// Secreto en memoria: se borra al soltarse y nunca se imprime.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub trait CredentialStore: Send + Sync {
    /// # Errors
    /// Error del sistema al escribir.
    fn put(&self, device_id: &str, secret: &Secret) -> Result<(), PlatformError>;

    /// # Errors
    /// Error del sistema al leer (ausencia no es error: devuelve `None`).
    fn get(&self, device_id: &str) -> Result<Option<Secret>, PlatformError>;

    /// # Errors
    /// Error del sistema al borrar.
    fn delete(&self, device_id: &str) -> Result<(), PlatformError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsCredentialStore;

fn target(device_id: &str) -> HSTRING {
    HSTRING::from(format!("Jarvis/WindowsAgent/{device_id}"))
}

impl CredentialStore for WindowsCredentialStore {
    fn put(&self, device_id: &str, secret: &Secret) -> Result<(), PlatformError> {
        let target = target(device_id);
        let user = HSTRING::from(device_id);
        let mut blob = secret.expose().as_bytes().to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_ptr().cast_mut()),
            UserName: PWSTR(user.as_ptr().cast_mut()),
            CredentialBlobSize: u32::try_from(blob.len()).unwrap_or(0),
            CredentialBlob: blob.as_mut_ptr(),
            // Solo esta máquina: el secreto no viaja con un perfil móvil.
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: la estructura y sus cadenas viven hasta el final de la llamada.
        let result = unsafe { CredWriteW(&credential, 0) };
        blob.zeroize();
        result.map_err(|e| PlatformError::new(PlatformErrorKind::Other, format!("CredWriteW: {e}")))
    }

    fn get(&self, device_id: &str) -> Result<Option<Secret>, PlatformError> {
        let target = target(device_id);
        // SAFETY: el buffer devuelto se libera con CredFree tras copiar el blob.
        unsafe {
            let mut raw = std::ptr::null_mut::<CREDENTIALW>();
            if CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &raw mut raw).is_err() {
                return Ok(None);
            }
            if raw.is_null() {
                return Ok(None);
            }
            let credential = &*raw;
            let len = credential.CredentialBlobSize as usize;
            let secret = if len == 0 || credential.CredentialBlob.is_null() {
                None
            } else {
                let bytes = std::slice::from_raw_parts(credential.CredentialBlob, len);
                Some(Secret::new(String::from_utf8_lossy(bytes).into_owned()))
            };
            CredFree(raw.cast());
            Ok(secret)
        }
    }

    fn delete(&self, device_id: &str) -> Result<(), PlatformError> {
        let target = target(device_id);
        // SAFETY: cadena válida terminada en NUL.
        unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) }
            .map_err(|e| PlatformError::new(PlatformErrorKind::NotFound, format!("CredDeleteW: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_never_prints_its_value() {
        let secret = Secret::new("CANARY-super-secret");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret}"), "***");
    }

    #[test]
    fn round_trips_through_credential_manager() {
        let store = WindowsCredentialStore;
        let device = format!("test-{}", std::process::id());
        let secret = Secret::new("s3cr3t-value");
        store.put(&device, &secret).expect("write");
        let read = store.get(&device).expect("read").expect("present");
        assert_eq!(read.expose(), secret.expose());
        store.delete(&device).expect("delete");
        assert!(store.get(&device).expect("read").is_none());
    }
}
