//! Named pipe local de la CLI con ACL restringida al SID del usuario (WT-11).
//!
//! Sin esta ACL, cualquier usuario de la máquina podría enviar comandos al agente (aprobar una
//! tarea R3, por ejemplo). También se rechazan clientes remotos.

use std::ffi::c_void;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::core::{HSTRING, PCWSTR};

/// Descriptor de seguridad propio: solo el usuario indicado y SYSTEM tienen acceso total.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: el descriptor lo asignó ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0.0)));
            }
        }
    }
}

fn descriptor_for(user_sid: &str) -> std::io::Result<SecurityDescriptor> {
    // D:P = DACL protegida (no hereda). GA = acceso total.
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;{user_sid})(A;;GA;;;SY)"));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` es una cadena válida terminada en NUL; el descriptor se libera en Drop.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &raw mut descriptor,
            None,
        )
        .map_err(|e| std::io::Error::other(format!("invalid security descriptor: {e}")))?;
    }
    Ok(SecurityDescriptor(descriptor))
}

/// Crea una instancia del servidor de named pipe con la ACL del usuario.
///
/// # Errors
/// Si el descriptor de seguridad o la creación del pipe fallan.
pub fn create_server(name: &str, user_sid: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let descriptor = descriptor_for(user_sid)?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: false.into(),
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true);
    // SAFETY: `attributes` vive durante la llamada y apunta a un descriptor válido.
    unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast::<c_void>()) }
}
