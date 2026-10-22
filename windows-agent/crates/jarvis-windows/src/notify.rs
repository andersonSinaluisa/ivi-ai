//! Aviso al usuario de que hay algo esperando su decisión (HU-WIN-027).
//!
//! Es un globo del área de notificación, **sin botones de acción**: el aviso solo dice qué hacer
//! (`jarvis-agent approvals approve …`). Aprobar desde un toast exigiría una UI firmada; mientras
//! no exista, otra aplicación podría imitarla para inducir clics (WT-15).

use std::ffi::c_void;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{IDI_INFORMATION, LoadIconW};

const ICON_ID: u32 = 1;
const CALLBACK_MESSAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// Icono del área de notificación asociado a la ventana *message-only* del agente.
#[derive(Debug)]
pub struct TrayIcon {
    hwnd: isize,
}

fn wide(text: &str, buffer: &mut [u16]) {
    let encoded: Vec<u16> = text.encode_utf16().take(buffer.len().saturating_sub(1)).collect();
    buffer[..encoded.len()].copy_from_slice(&encoded);
    buffer[encoded.len()] = 0;
}

impl TrayIcon {
    /// Registra el icono. `hwnd` es el handle de [`crate::events::MessageWindow`].
    ///
    /// # Errors
    /// Si Windows rechaza añadir el icono (por ejemplo, sin sesión interactiva).
    pub fn new(hwnd: isize, tooltip: &str) -> Result<Self, PlatformError> {
        let mut data = NOTIFYICONDATAW {
            cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
            hWnd: HWND(hwnd as *mut c_void),
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: CALLBACK_MESSAGE,
            ..Default::default()
        };
        wide(tooltip, &mut data.szTip);
        // SAFETY: estructura completa con su tamaño; el icono es uno del sistema.
        unsafe {
            data.hIcon = LoadIconW(None, IDI_INFORMATION).unwrap_or_default();
            if !Shell_NotifyIconW(NIM_ADD, &raw const data).as_bool() {
                return Err(PlatformError::new(PlatformErrorKind::Other, "Shell_NotifyIconW(NIM_ADD) failed"));
            }
        }
        Ok(Self { hwnd })
    }

    /// Muestra un globo informativo. Sin acciones: solo texto.
    ///
    /// # Errors
    /// Si Windows rechaza mostrarlo (notificaciones desactivadas, modo concentración…).
    pub fn balloon(&self, title: &str, body: &str) -> Result<(), PlatformError> {
        let mut data = NOTIFYICONDATAW {
            cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
            hWnd: HWND(self.hwnd as *mut c_void),
            uID: ICON_ID,
            uFlags: NIF_INFO,
            ..Default::default()
        };
        data.dwInfoFlags = NIIF_INFO;
        wide(title, &mut data.szInfoTitle);
        wide(body, &mut data.szInfo);
        // SAFETY: estructura completa con su tamaño y cadenas terminadas en NUL.
        unsafe {
            if Shell_NotifyIconW(NIM_MODIFY, &raw const data).as_bool() {
                Ok(())
            } else {
                Err(PlatformError::new(PlatformErrorKind::Other, "Shell_NotifyIconW(NIM_MODIFY) failed"))
            }
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let data = NOTIFYICONDATAW {
            cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
            hWnd: HWND(self.hwnd as *mut c_void),
            uID: ICON_ID,
            ..Default::default()
        };
        // SAFETY: el icono lo añadió este tipo con el mismo hwnd y uID.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &raw const data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::MessageWindow;

    #[tokio::test]
    async fn adds_and_removes_a_tray_icon() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let window = MessageWindow::spawn(tx).expect("window");
        match TrayIcon::new(window.handle(), "Jarvis (test)") {
            Ok(icon) => {
                // El globo puede no mostrarse (modo concentración): no se exige, solo que no rompa.
                let _ = icon.balloon("Jarvis", "test");
                drop(icon);
            }
            // Sin sesión interactiva (CI headless) no hay área de notificación.
            Err(error) => println!("SKIPPED: {error}"),
        }
    }
}
