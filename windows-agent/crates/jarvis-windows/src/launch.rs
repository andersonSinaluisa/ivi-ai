//! Lanzamiento de aplicaciones: Win32 (`ShellExecuteExW`) y apps empaquetadas (AUMID).
//!
//! Nunca se pasan argumentos: el input remoto solo elige una entrada del catálogo (ADR-008).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jarvis_tooling::platform::{Launch, LaunchTarget, LaunchedProcess, PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::{ERROR_ELEVATION_REQUIRED, GetLastError, HANDLE};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::{
    ACTIVATEOPTIONS, ApplicationActivationManager, IApplicationActivationManager, SEE_MASK_NOASYNC,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use crate::sys::{OwnedHandle, wide_to_string};

const STILL_ACTIVE: u32 = 259;

/// Proceso lanzado del que conservamos el handle para poder leer su código de salida.
#[derive(Debug)]
pub struct Win32Process {
    handle: Option<OwnedHandle>,
    pid: u32,
}

impl LaunchedProcess for Win32Process {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn exit_code(&self) -> Option<u32> {
        let handle = self.handle.as_ref()?;
        let mut code = 0u32;
        // SAFETY: handle propio y válido mientras vive `self`.
        let ok = unsafe { GetExitCodeProcess(handle.0, &raw mut code) };
        if ok.is_ok() && code != STILL_ACTIVE { Some(code) } else { None }
    }
}

/// Resuelve un nombre registrado en `App Paths` (HKCU primero, luego HKLM) a una ruta existente.
#[must_use]
pub fn resolve_app_path(exe_name: &str) -> Option<PathBuf> {
    let subkey = HSTRING::from(format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe_name}"));
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buffer = vec![0u16; 1024];
        let mut size = u32::try_from(buffer.len() * 2).unwrap_or(2048);
        // SAFETY: buffer propio con su tamaño en bytes; la clave es una cadena terminada en NUL.
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR(subkey.as_ptr()),
                PCWSTR::null(),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&raw mut size),
            )
        };
        if status.is_ok() {
            let raw = wide_to_string(&buffer);
            let path = PathBuf::from(raw.trim_matches('"'));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Lanza una app del catálogo.
///
/// # Errors
/// `NotFound` si el ejecutable no existe, `ElevationRequired` si pide UAC (no se reintenta con
/// `runas`), `AccessDenied` u `Other` en el resto.
pub fn launch(target: &LaunchTarget) -> Result<Launch, PlatformError> {
    match target {
        LaunchTarget::Executable(path) => launch_executable(path),
        LaunchTarget::PackagedApp(aumid) => launch_packaged(aumid),
    }
}

fn launch_executable(path: &Path) -> Result<Launch, PlatformError> {
    if !path.is_file() {
        return Err(PlatformError::new(PlatformErrorKind::NotFound, "executable not found"));
    }
    let file = HSTRING::from(path.as_os_str());
    let directory = path.parent().map(|p| HSTRING::from(p.as_os_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: directory.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let launched_at = SystemTime::now();
    // SAFETY: `info` está completamente inicializada y las cadenas viven hasta el final de la llamada.
    let result = unsafe { ShellExecuteExW(&raw mut info) };
    if let Err(error) = result {
        // SAFETY: lectura del último error del hilo.
        let code = unsafe { GetLastError() };
        if code == ERROR_ELEVATION_REQUIRED {
            return Err(PlatformError::new(
                PlatformErrorKind::ElevationRequired,
                "the application requires elevation (UAC); the agent never elevates",
            ));
        }
        return Err(PlatformError::new(PlatformErrorKind::Other, format!("ShellExecuteExW: {error}")));
    }
    let handle = (!info.hProcess.is_invalid()).then(|| OwnedHandle(info.hProcess));
    let pid = handle.as_ref().map_or(0, |h| process_id(h.0));
    Ok(Launch { process: Some(Box::new(Win32Process { handle, pid })), launched_at })
}

fn process_id(handle: HANDLE) -> u32 {
    // SAFETY: handle válido; GetProcessId no tiene efectos.
    unsafe { windows::Win32::System::Threading::GetProcessId(handle) }
}

fn launch_packaged(aumid: &str) -> Result<Launch, PlatformError> {
    let app_id = HSTRING::from(aumid);
    let launched_at = SystemTime::now();
    // SAFETY: COM se inicializa y se libera en este hilo (las tools llaman desde spawn_blocking);
    // el puntero de salida `pid` es local.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| PlatformError::new(PlatformErrorKind::Other, format!("CoCreateInstance: {e}")))?;
            let pid = manager
                .ActivateApplication(PCWSTR(app_id.as_ptr()), PCWSTR::null(), ACTIVATEOPTIONS(0))
                .map_err(|e| PlatformError::new(PlatformErrorKind::NotFound, format!("ActivateApplication: {e}")))?;
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().map(OwnedHandle);
            Ok(Launch { process: Some(Box::new(Win32Process { handle, pid })), launched_at })
        })();
        if com.is_ok() {
            CoUninitialize();
        }
        result
    }
}

/// Ubicaciones donde un ejecutable se considera legítimo.
///
/// Se aceptan las carpetas del sistema y de programas, y las instalaciones por usuario (Chrome,
/// VS Code y otras viven en el perfil). Se rechazan siempre las carpetas de paso donde cae
/// cualquier descarga (`%TEMP%`, `Downloads`): ahí es donde aparecería un `chrome.exe` impostor
/// (WT-09) y desde donde se intentaría un secuestro de ejecución (WT-08).
#[must_use]
pub fn is_trusted_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let lower = path.to_string_lossy().to_lowercase();
    let under = |var: &str| {
        std::env::var(var)
            .ok()
            .filter(|root| !root.is_empty())
            .is_some_and(|root| lower.starts_with(&root.to_lowercase()))
    };
    let staging = under("TEMP")
        || under("TMP")
        || std::env::var("USERPROFILE")
            .ok()
            .is_some_and(|profile| lower.starts_with(&format!("{}\\downloads", profile.to_lowercase())));
    if staging {
        return false;
    }
    under("SystemRoot") || under("ProgramFiles") || under("ProgramFiles(x86)") || under("UserProfile")
}

/// Error de último recurso para rutas no confiables.
#[must_use]
pub fn untrusted_error() -> PlatformError {
    PlatformError::new(PlatformErrorKind::AccessDenied, "executable is not in a trusted location")
}
