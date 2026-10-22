//! Ventana *message-only* del agente: recibe los avisos de Windows que el proceso necesita para
//! comportarse bien (HU-WIN-004, HU-WIN-009, HU-WIN-011).
//!
//! - `WM_QUERYENDSESSION` / `WM_ENDSESSION`: apagado o cierre de sesión → apagado ordenado.
//! - `WM_WTSSESSION_CHANGE`: bloqueo, desbloqueo, conexión y desconexión de la sesión.
//! - `WM_POWERBROADCAST`: suspensión y reanudación (para reconectar de inmediato).
//!
//! La ventana vive en su propio hilo con un bucle de mensajes; el resto del agente solo recibe
//! eventos por un canal.

use std::ffi::c_void;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterSuspendResumeNotification;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
    PostMessageW, RegisterClassW, SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_CREATE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_WTSSESSION_CHANGE, WNDCLASSW,
    WTS_CONSOLE_CONNECT, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
    WTS_SESSION_LOGOFF, WTS_SESSION_UNLOCK,
};
use windows::core::{HSTRING, PCWSTR};

/// Avisos del sistema que le importan al agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    /// Windows pregunta si puede cerrar la sesión o apagar: hay que terminar de forma ordenada.
    EndSessionRequested,
    SessionLocked,
    SessionUnlocked,
    SessionDisconnected,
    SessionConnected,
    Suspending,
    /// Reanudación tras suspensión: conviene reconectar sin esperar al backoff.
    Resumed,
}

/// Ventana viva; al soltarse, cierra el hilo y la ventana.
#[derive(Debug)]
pub struct MessageWindow {
    hwnd: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MessageWindow {
    /// Crea la ventana en un hilo propio y empieza a enviar eventos por el canal.
    ///
    /// # Errors
    /// Si Windows rechaza registrar la clase o crear la ventana.
    pub fn spawn(sender: UnboundedSender<SystemEvent>) -> Result<Self, PlatformError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<isize, String>>();
        let thread = std::thread::Builder::new()
            .name("jarvis-window".to_owned())
            .spawn(move || run_message_loop(&sender, &ready_tx))
            .map_err(|e| PlatformError::new(PlatformErrorKind::Other, e.to_string()))?;

        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(hwnd)) => Ok(Self { hwnd, thread: Some(thread) }),
            Ok(Err(error)) => Err(PlatformError::new(PlatformErrorKind::Other, error)),
            Err(error) => Err(PlatformError::new(PlatformErrorKind::Other, format!("window thread: {error}"))),
        }
    }

    /// Handle de la ventana (lo usa la bandeja de notificaciones).
    #[must_use]
    pub const fn handle(&self) -> isize {
        self.hwnd
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        if self.hwnd != 0 {
            // SAFETY: el handle lo creó este tipo; WM_CLOSE hace que el bucle termine.
            unsafe {
                let _ = PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

const CLASS_NAME: &str = "JarvisAgentMessageWindow";

fn run_message_loop(sender: &UnboundedSender<SystemEvent>, ready: &std::sync::mpsc::Sender<Result<isize, String>>) {
    // El puntero al sender se guarda en la ventana: el wndproc es una función C sin estado propio.
    let boxed: Box<UnboundedSender<SystemEvent>> = Box::new(sender.clone());
    let class = HSTRING::from(CLASS_NAME);

    // SAFETY: la clase y la ventana se crean con parámetros válidos y se destruyen al salir del
    // bucle; el sender vive hasta WM_DESTROY, donde se libera.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // Registrar dos veces la misma clase devuelve 0 y no es un error si ya existe.
        RegisterClassW(&wnd_class);

        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(class.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            Some(Box::into_raw(boxed).cast::<c_void>()),
        )
    };

    let hwnd = match hwnd {
        Ok(hwnd) if !hwnd.is_invalid() => hwnd,
        Ok(_) | Err(_) => {
            let _ = ready.send(Err("CreateWindowExW failed".to_owned()));
            return;
        }
    };

    // SAFETY: `hwnd` es válido; ambas suscripciones se liberan al destruir la ventana.
    unsafe {
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_err() {
            tracing::warn!("cannot subscribe to session notifications; falling back to polling");
        }
        if RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE).is_err() {
            tracing::warn!("cannot subscribe to power notifications");
        }
    }

    let _ = ready.send(Ok(hwnd.0 as isize));

    // SAFETY: bucle de mensajes estándar sobre la ventana de este hilo.
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
        let _ = WTSUnRegisterSessionNotification(hwnd);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: el puntero al sender se guarda en WM_CREATE y se libera en WM_DESTROY.
    unsafe {
        match message {
            WM_CREATE => {
                let create = lparam.0 as *const CREATESTRUCTW;
                if !create.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
                LRESULT(0)
            }
            WM_WTSSESSION_CHANGE => {
                let event = match u32::try_from(wparam.0).unwrap_or(0) {
                    WTS_SESSION_LOCK => Some(SystemEvent::SessionLocked),
                    WTS_SESSION_UNLOCK => Some(SystemEvent::SessionUnlocked),
                    WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT | WTS_SESSION_LOGOFF => {
                        Some(SystemEvent::SessionDisconnected)
                    }
                    WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => Some(SystemEvent::SessionConnected),
                    _ => None,
                };
                if let Some(event) = event {
                    emit(hwnd, event);
                }
                LRESULT(0)
            }
            WM_POWERBROADCAST => {
                match u32::try_from(wparam.0).unwrap_or(0) {
                    PBT_APMSUSPEND => emit(hwnd, SystemEvent::Suspending),
                    PBT_APMRESUMEAUTOMATIC => emit(hwnd, SystemEvent::Resumed),
                    _ => {}
                }
                LRESULT(1)
            }
            WM_QUERYENDSESSION => {
                // Se acepta el cierre y se avisa al agente para que termine de forma ordenada.
                emit(hwnd, SystemEvent::EndSessionRequested);
                LRESULT(1)
            }
            WM_ENDSESSION => {
                emit(hwnd, SystemEvent::EndSessionRequested);
                LRESULT(0)
            }
            WM_DESTROY => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if pointer != 0 {
                    drop(Box::from_raw(pointer as *mut UnboundedSender<SystemEvent>));
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// Envía un evento por el canal guardado en la ventana.
unsafe fn emit(hwnd: HWND, event: SystemEvent) {
    // SAFETY: el puntero lo guardó WM_CREATE y sigue vivo hasta WM_DESTROY.
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if pointer == 0 {
            return;
        }
        let sender = &*(pointer as *const UnboundedSender<SystemEvent>);
        let _ = sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La ventana se crea, queda viva y se cierra sin bloquear el proceso.
    #[tokio::test]
    async fn creates_and_closes_the_message_window() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let window = MessageWindow::spawn(tx).expect("window");
        assert_ne!(window.handle(), 0);

        // Un WM_CLOSE cierra la ventana y termina el hilo (lo hace Drop).
        drop(window);
        assert!(rx.try_recv().is_err(), "sin eventos del sistema durante el test");
    }
}
