//! Estado de la sesión de Windows (docs/05-windows/session-model.md).
//!
//! MVP: sondeo (no notificaciones de ventana). `Locked` y `Disconnected` se obtienen de WTS y
//! `Idle` de `GetLastInputInfo`.

use std::ffi::c_void;
use std::time::Duration;

use jarvis_tooling::platform::SessionState;
use windows::Win32::System::RemoteDesktop::{
    WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSActive, WTSConnectState, WTSFreeMemory,
    WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

const WTS_SESSIONSTATE_LOCK: u32 = 0;
const WTS_SESSIONSTATE_UNKNOWN: u32 = 0xFFFF_FFFF;

/// Estado actual de la sesión del agente.
#[must_use]
pub fn state(idle_threshold: Duration) -> SessionState {
    if connect_state().is_some_and(|s| s != WTSActive) {
        return SessionState::Disconnected;
    }
    match locked() {
        Some(true) => SessionState::Locked,
        _ => {
            if idle_for().is_some_and(|idle| idle >= idle_threshold) {
                SessionState::Idle
            } else {
                SessionState::Active
            }
        }
    }
}

fn connect_state() -> Option<WTS_CONNECTSTATE_CLASS> {
    // SAFETY: se pide un valor de tamaño fijo y se libera con WTSFreeMemory.
    unsafe {
        let mut buffer = windows::core::PWSTR::null();
        let mut bytes = 0u32;
        let ok = WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            WTSConnectState,
            &raw mut buffer,
            &raw mut bytes,
        );
        if ok.is_err() || buffer.is_null() || (bytes as usize) < size_of::<WTS_CONNECTSTATE_CLASS>() {
            return None;
        }
        let value = *buffer.0.cast::<WTS_CONNECTSTATE_CLASS>();
        WTSFreeMemory(buffer.0.cast::<c_void>());
        Some(value)
    }
}

/// `Some(true)` si la estación está bloqueada; `None` si Windows no lo informa.
#[must_use]
pub fn locked() -> Option<bool> {
    // SAFETY: WTSINFOEXW se copia antes de liberar el buffer devuelto por Windows.
    unsafe {
        let mut buffer = windows::core::PWSTR::null();
        let mut bytes = 0u32;
        let ok = WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &raw mut buffer,
            &raw mut bytes,
        );
        if ok.is_err() || buffer.is_null() || (bytes as usize) < size_of::<WTSINFOEXW>() {
            return None;
        }
        let info = *buffer.0.cast::<WTSINFOEXW>();
        WTSFreeMemory(buffer.0.cast::<c_void>());
        let flags = info.Data.WTSInfoExLevel1.SessionFlags;
        let flags = u32::from_ne_bytes(flags.to_ne_bytes());
        match flags {
            WTS_SESSIONSTATE_LOCK => Some(true),
            WTS_SESSIONSTATE_UNKNOWN => None,
            _ => Some(false),
        }
    }
}

/// Tiempo sin entrada del usuario.
#[must_use]
pub fn idle_for() -> Option<Duration> {
    let mut info = LASTINPUTINFO { cbSize: u32::try_from(size_of::<LASTINPUTINFO>()).unwrap_or(0), dwTime: 0 };
    // SAFETY: estructura local con su tamaño correcto.
    let ok = unsafe { GetLastInputInfo(&raw mut info) };
    if !ok.as_bool() {
        return None;
    }
    // SAFETY: sin precondiciones.
    let now = unsafe { GetTickCount64() };
    Some(Duration::from_millis(now.saturating_sub(u64::from(info.dwTime))))
}
