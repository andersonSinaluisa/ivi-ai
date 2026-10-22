//! Ventanas: enumeración de ventanas top-level reales, ventana activa y cierre ordenado.

use std::ffi::c_void;

use jarvis_tooling::platform::{ForegroundWindow, PlatformError, WindowInfo};
use windows::Win32::Foundation::{HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PostMessageW, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow, WM_CLOSE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::sys::{last_error, wide_to_string};

struct Collector {
    windows: Vec<WindowInfo>,
}

/// Ventana "real" para el usuario: visible, con título, sin owner, no *cloaked* y no tool window.
fn is_real_window(hwnd: HWND) -> bool {
    // SAFETY: `hwnd` proviene de EnumWindows; todas las llamadas aceptan handles inválidos devolviendo error.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        if !GetWindow(hwnd, GW_OWNER).unwrap_or_default().is_invalid() {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex_style & (WS_EX_TOOLWINDOW.0 as isize) != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let size = u32::try_from(size_of::<u32>()).unwrap_or(4);
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast::<c_void>(), size).is_ok() && cloaked != 0
        {
            return false;
        }
        true
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` es el puntero al Collector que pasó `top_level_windows`, vivo durante la enumeración.
    let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
    if is_real_window(hwnd) {
        let mut pid = 0u32;
        // SAFETY: `hwnd` válido; `pid` es una salida.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid != 0 {
            let mut class = [0u16; 256];
            // SAFETY: buffer propio del tamaño indicado.
            let len = unsafe { GetClassNameW(hwnd, &mut class) };
            collector.windows.push(WindowInfo {
                handle: hwnd.0 as isize,
                pid,
                class_name: if len > 0 { wide_to_string(&class) } else { String::new() },
            });
        }
    }
    TRUE
}

/// Enumera las ventanas top-level reales.
///
/// # Errors
/// Si falla `EnumWindows`.
pub fn top_level_windows() -> Result<Vec<WindowInfo>, PlatformError> {
    let mut collector = Collector { windows: Vec::new() };
    // SAFETY: el puntero al collector vive durante toda la llamada.
    unsafe {
        EnumWindows(Some(enum_proc), LPARAM((&raw mut collector) as isize)).map_err(|_| last_error("EnumWindows"))?;
    }
    Ok(collector.windows)
}

/// Ventana en primer plano. `None` con la sesión bloqueada o en el Secure Desktop.
///
/// # Errors
/// No falla en la práctica; se mantiene el `Result` por simetría con la interfaz.
pub fn foreground(include_title: bool) -> Result<Option<ForegroundWindow>, PlatformError> {
    // SAFETY: llamadas de solo lectura sobre el handle devuelto por el sistema.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return Ok(None);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
        if pid == 0 {
            return Ok(None);
        }
        let title = include_title.then(|| {
            let mut buffer = vec![0u16; 512];
            let len = GetWindowTextW(hwnd, &mut buffer);
            (len > 0).then(|| wide_to_string(&buffer))
        });
        Ok(Some(ForegroundWindow { pid, process_name: None, title: title.flatten() }))
    }
}

/// Envía `WM_CLOSE` a las ventanas top-level de esos procesos (cierre ordenado: la app puede guardar).
///
/// # Errors
/// Si falla la enumeración de ventanas.
pub fn request_close(pids: &[u32]) -> Result<usize, PlatformError> {
    let mut signalled = 0;
    for window in top_level_windows()? {
        if pids.contains(&window.pid) {
            // SAFETY: handle obtenido de la enumeración; PostMessage no bloquea ni toma ownership.
            let posted =
                unsafe { PostMessageW(Some(HWND(window.handle as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            if posted.is_ok() {
                signalled += 1;
            }
        }
    }
    Ok(signalled)
}

/// PID de la ventana en primer plano, sin leer el título.
///
/// # Errors
/// No falla en la práctica; se mantiene el `Result` por simetría con la interfaz.
pub fn foreground_pid() -> Result<Option<u32>, PlatformError> {
    Ok(foreground(false)?.map(|window| window.pid))
}

/// Trae al frente la primera ventana top-level de esos procesos.
///
/// Devuelve `true` solo si al terminar **la ventana está realmente en primer plano**. Windows
/// restringe `SetForegroundWindow` a procesos que hayan recibido entrada reciente: cuando lo
/// deniega, el sistema se limita a parpadear el botón de la barra de tareas. En ese caso esto
/// devuelve `false` en vez de dar por buena la operación (la alternativa sería sintetizar pulsaciones
/// de teclado, que ADR-002 descarta).
///
/// # Errors
/// Si falla la enumeración de ventanas.
pub fn focus(pids: &[u32]) -> Result<bool, PlatformError> {
    let Some(window) = top_level_windows()?.into_iter().find(|window| pids.contains(&window.pid)) else {
        return Ok(false);
    };
    let hwnd = HWND(window.handle as *mut c_void);
    // SAFETY: handle recién obtenido de la enumeración; ninguna de estas llamadas toma ownership.
    unsafe {
        let command = if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW };
        let _ = ShowWindow(hwnd, command);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        Ok(GetForegroundWindow() == hwnd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_only_real_windows() {
        let windows = top_level_windows().expect("enumerate");
        for window in &windows {
            assert_ne!(window.pid, 0, "toda ventana tiene un proceso");
            assert!(window.handle != 0);
        }
    }

    #[test]
    fn focusing_an_unknown_process_does_nothing() {
        // PID que no existe: no hay ventana que enfocar y no se inventa un éxito.
        assert!(!focus(&[u32::MAX]).expect("focus"));
    }

    #[test]
    fn foreground_pid_matches_the_foreground_window() {
        let by_pid = foreground_pid().expect("pid");
        let full = foreground(false).expect("foreground");
        assert_eq!(by_pid, full.map(|window| window.pid));
    }
}
