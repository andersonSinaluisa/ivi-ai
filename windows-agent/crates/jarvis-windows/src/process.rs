//! Enumeración y control de procesos (docs/05-windows/process-management.md).
//!
//! Solo se ven procesos de la **sesión y el usuario del agente**. Nunca se lee línea de comandos,
//! entorno ni memoria de otros procesos.

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind, ProcessInfo};
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
};
use windows::core::PWSTR;

use crate::sys::{OwnedHandle, last_error, wide_to_string};

/// Procesos que nunca se cierran, aunque estén en el catálogo (docs/03-tools/process-tools.md#nunca-matar).
pub const PROTECTED_PROCESSES: &[&str] = &[
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "lsass.exe",
    "services.exe",
    "smss.exe",
    "svchost.exe",
    "dwm.exe",
    "MsMpEng.exe",
    "jarvis-agent.exe",
];

#[must_use]
pub fn is_protected(name: &str) -> bool {
    PROTECTED_PROCESSES.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// Identidad del proceso actual, para filtrar lo que se observa.
#[derive(Debug, Clone)]
pub struct OwnIdentity {
    pub session_id: u32,
    pub user_sid: String,
}

impl OwnIdentity {
    /// # Errors
    /// Si no se puede consultar la sesión o el SID.
    pub fn current() -> Result<Self, PlatformError> {
        // SAFETY: se consulta la sesión del propio PID.
        let session_id = unsafe {
            let mut session = 0u32;
            ProcessIdToSessionId(GetCurrentProcessId(), &mut session)
                .map_err(|_| last_error("ProcessIdToSessionId"))?;
            session
        };
        Ok(Self { session_id, user_sid: crate::sys::user_sid_string()? })
    }
}

fn filetime_to_system_time(ft: FILETIME) -> Option<SystemTime> {
    let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    // 100 ns desde 1601-01-01; 11644473600 s hasta la época Unix.
    let unix_ticks = ticks.checked_sub(116_444_736_000_000_000)?;
    Some(UNIX_EPOCH + Duration::from_nanos(unix_ticks.saturating_mul(100)))
}

fn token_sid_and_elevation(process: HANDLE) -> (Option<String>, Option<bool>) {
    // SAFETY: `process` es un handle abierto con PROCESS_QUERY_LIMITED_INFORMATION; los buffers se
    // dimensionan con la primera llamada a GetTokenInformation.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_err() {
            return (None, None); // Normalmente un proceso elevado: no consultable desde integridad media.
        }
        let token = OwnedHandle(token);

        let mut size = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
        let mut sid_text = None;
        if size > 0 {
            let mut buffer = vec![0u8; size as usize];
            if GetTokenInformation(token.0, TokenUser, Some(buffer.as_mut_ptr().cast::<c_void>()), size, &mut size)
                .is_ok()
            {
                let user = buffer.as_ptr().cast::<TOKEN_USER>();
                let mut text = PWSTR::null();
                if ConvertSidToStringSidW((*user).User.Sid, &mut text).is_ok() {
                    sid_text = text.to_string().ok();
                    let _ = LocalFree(Some(HLOCAL(text.0.cast::<c_void>())));
                }
            }
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut elevation_size = u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(4);
        let elevated = GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&raw mut elevation).cast::<c_void>()),
            elevation_size,
            &mut elevation_size,
        )
        .is_ok()
        .then_some(elevation.TokenIsElevated != 0);
        (sid_text, elevated)
    }
}

/// Enumera los procesos de la sesión y el usuario del agente.
///
/// # Errors
/// Si falla la instantánea de Toolhelp.
pub fn list(identity: &OwnIdentity) -> Result<Vec<ProcessInfo>, PlatformError> {
    let mut out = Vec::new();
    // SAFETY: la instantánea se cierra al soltar `snapshot`; `entry` lleva su dwSize obligatorio.
    unsafe {
        let snapshot = OwnedHandle(
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|_| last_error("CreateToolhelp32Snapshot"))?,
        );
        let mut entry =
            PROCESSENTRY32W { dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0), ..Default::default() };
        if Process32FirstW(snapshot.0, &mut entry).is_err() {
            return Ok(out);
        }
        loop {
            let pid = entry.th32ProcessID;
            if pid != 0 {
                let mut session = u32::MAX;
                let same_session = ProcessIdToSessionId(pid, &mut session).is_ok() && session == identity.session_id;
                if same_session && let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let handle = OwnedHandle(handle);
                    let (sid, elevated) = token_sid_and_elevation(handle.0);
                    // Procesos de otro usuario quedan fuera; si el token no es consultable (elevado)
                    // se conserva la entrada con `elevated: None`.
                    if sid.as_deref().is_none_or(|s| s == identity.user_sid) {
                        let mut buffer = vec![0u16; 1024];
                        let mut len = u32::try_from(buffer.len()).unwrap_or(1024);
                        let image_path = QueryFullProcessImageNameW(
                            handle.0,
                            PROCESS_NAME_FORMAT(0),
                            PWSTR(buffer.as_mut_ptr()),
                            &mut len,
                        )
                        .is_ok()
                        .then(|| PathBuf::from(wide_to_string(&buffer)));

                        let mut created = FILETIME::default();
                        let (mut exit, mut kernel, mut user) =
                            (FILETIME::default(), FILETIME::default(), FILETIME::default());
                        let created_at = GetProcessTimes(handle.0, &mut created, &mut exit, &mut kernel, &mut user)
                            .is_ok()
                            .then(|| filetime_to_system_time(created))
                            .flatten();

                        out.push(ProcessInfo {
                            pid,
                            parent_pid: entry.th32ParentProcessID,
                            name: wide_to_string(&entry.szExeFile),
                            image_path,
                            created_at,
                            // None = token no consultable (proceso elevado): las tools lo tratan como no controlable.
                            elevated,
                        });
                    }
                }
            }
            if Process32NextW(snapshot.0, &mut entry).is_err() {
                break;
            }
        }
    }
    Ok(out)
}

/// Termina un proceso comprobando su hora de creación (evita matar un PID reutilizado).
///
/// # Errors
/// `AccessDenied` si el proceso está elevado o protegido.
pub fn terminate(pid: u32, created_at: Option<SystemTime>) -> Result<(), PlatformError> {
    // SAFETY: se abre el proceso con los permisos mínimos y el handle se cierra en Drop.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|_| last_error("OpenProcess(PROCESS_TERMINATE)"))?;
        let handle = OwnedHandle(handle);
        if let Some(expected) = created_at {
            let mut created = FILETIME::default();
            let (mut exit, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetProcessTimes(handle.0, &mut created, &mut exit, &mut kernel, &mut user).is_ok()
                && let Some(actual) = filetime_to_system_time(created)
                && actual != expected
            {
                return Err(PlatformError::new(PlatformErrorKind::NotFound, "process id was reused"));
            }
        }
        TerminateProcess(handle.0, 1).map_err(|_| last_error("TerminateProcess"))
    }
}
