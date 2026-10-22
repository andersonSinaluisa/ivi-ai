//! Utilidades del sistema: handles RAII, SID del usuario, elevación, rutas conocidas, instancia única.

use std::ffi::c_void;
use std::path::PathBuf;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// Handle propio que se cierra al soltarse.
#[derive(Debug)]
pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: el handle lo abrió este tipo y no se cierra dos veces (Drop se ejecuta una vez).
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: los handles de kernel de Windows son válidos en todo el proceso y las APIs usadas
// (GetExitCodeProcess, TerminateProcess, CloseHandle) son seguras entre hilos.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

pub(crate) fn last_error(context: &str) -> PlatformError {
    // SAFETY: GetLastError no tiene precondiciones.
    let code = unsafe { GetLastError() };
    let kind = match code.0 {
        2 | 3 => PlatformErrorKind::NotFound,
        5 => PlatformErrorKind::AccessDenied,
        740 => PlatformErrorKind::ElevationRequired,
        _ => PlatformErrorKind::Other,
    };
    PlatformError::new(kind, format!("{context} failed (error {})", code.0))
}

pub(crate) fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Expande `%VARIABLES%` de entorno.
#[must_use]
pub fn expand_env(text: &str) -> String {
    let input = HSTRING::from(text);
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: `input` es una cadena terminada en NUL y `buffer` tiene la capacidad indicada.
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(input.as_ptr()), Some(&mut buffer)) };
    if written == 0 { text.to_owned() } else { wide_to_string(&buffer) }
}

/// `%LOCALAPPDATA%` por la API de carpetas conocidas (no por variables de entorno).
///
/// # Errors
/// Si Windows no devuelve la carpeta.
pub fn local_app_data() -> Result<PathBuf, PlatformError> {
    // SAFETY: se pasa un FOLDERID válido y se libera el buffer devuelto.
    unsafe {
        let raw: PWSTR = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None)
            .map_err(|e| PlatformError::new(PlatformErrorKind::Other, format!("SHGetKnownFolderPath: {e}")))?;
        let path = raw.to_string().map_err(|e| PlatformError::new(PlatformErrorKind::Other, e.to_string()))?;
        CoTaskMemFree(Some(raw.0.cast::<c_void>()));
        Ok(PathBuf::from(path))
    }
}

/// SID del usuario actual en formato texto (`S-1-5-21-…`).
///
/// # Errors
/// Si no se puede consultar el token del proceso.
pub fn user_sid_string() -> Result<String, PlatformError> {
    // SAFETY: se abre el token del propio proceso y se consulta su tamaño antes de leerlo.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|_| last_error("OpenProcessToken"))?;
        let token = OwnedHandle(token);
        let mut size = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u8; size as usize];
        GetTokenInformation(token.0, TokenUser, Some(buffer.as_mut_ptr().cast::<c_void>()), size, &mut size)
            .map_err(|_| last_error("GetTokenInformation(TokenUser)"))?;
        let user = buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW((*user).User.Sid, &mut text).map_err(|_| last_error("ConvertSidToStringSidW"))?;
        let sid = text.to_string().map_err(|e| PlatformError::new(PlatformErrorKind::Other, e.to_string()))?;
        let _ = LocalFree(Some(HLOCAL(text.0.cast::<c_void>())));
        Ok(sid)
    }
}

/// `true` si el proceso corre con token elevado (el agente se niega a arrancar así).
#[must_use]
pub fn is_elevated() -> bool {
    // SAFETY: se consulta el token del propio proceso con el tamaño exacto de la estructura.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(4);
        let ok =
            GetTokenInformation(token.0, TokenElevation, Some((&raw mut elevation).cast::<c_void>()), size, &mut size);
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Mutex nombrado por sesión y usuario: impide dos agentes a la vez.
#[derive(Debug)]
pub struct SingleInstance {
    _handle: OwnedHandle,
}

impl SingleInstance {
    /// Adquiere el mutex `Local\Jarvis.Agent.<SID>`.
    ///
    /// # Errors
    /// Si ya existe otra instancia o no se puede crear el mutex.
    pub fn acquire(sid: &str) -> Result<Self, PlatformError> {
        let name = HSTRING::from(format!("Local\\Jarvis.Agent.{sid}"));
        // SAFETY: nombre válido terminado en NUL; el handle se cierra en Drop.
        unsafe {
            let handle = CreateMutexW(None, true, PCWSTR(name.as_ptr())).map_err(|_| last_error("CreateMutexW"))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Err(PlatformError::new(PlatformErrorKind::AccessDenied, "another agent instance is running"));
            }
            Ok(Self { _handle: OwnedHandle(handle) })
        }
    }
}
