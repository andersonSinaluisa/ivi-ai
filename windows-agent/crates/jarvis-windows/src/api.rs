//! Implementación de [`WindowsApi`] sobre Win32.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use jarvis_tooling::platform::{
    CancelFlag, CommandOutput, CommandSpec, ForegroundWindow, Launch, LaunchTarget, Opened, PlatformError,
    PlatformErrorKind, ProcessInfo, SessionState, UiElement, UiSelector, UiWindow, WindowInfo, WindowsApi,
};

use crate::notify::TrayIcon;
use crate::process::OwnIdentity;
use crate::uia::{Selector, UiaHost};
use crate::{commands, fs, launch, process, session, sys, window};

/// Acceso real a Windows. Todas las llamadas son bloqueantes: las tools las ejecutan en
/// `spawn_blocking`.
#[derive(Debug, Clone)]
pub struct Win32Api {
    identity: OwnIdentity,
    idle_threshold: Duration,
    /// Se rellena cuando el runtime crea la ventana *message-only*: antes de eso no hay dónde
    /// mostrar globos y `show_notification` falla en vez de fingir que avisó.
    tray: Arc<OnceLock<Arc<TrayIcon>>>,
    /// Host de UI Automation: el hilo STA se crea la primera vez que hace falta.
    uia: Arc<UiaHost>,
}

/// Selector del contrato de tools traducido al del host UIA.
fn selector_of(selector: &UiSelector) -> Selector {
    Selector {
        automation_id: selector.automation_id.clone(),
        name: selector.name.clone(),
        class_name: selector.class_name.clone(),
        control_type: selector.control_type.clone(),
    }
}

impl Win32Api {
    /// # Errors
    /// Si no se puede determinar la sesión o el usuario del proceso.
    pub fn new(idle_threshold: Duration) -> Result<Self, PlatformError> {
        Ok(Self {
            identity: OwnIdentity::current()?,
            idle_threshold,
            tray: Arc::new(OnceLock::new()),
            uia: Arc::new(UiaHost::new()),
        })
    }

    /// Asocia el icono del área de notificación (una sola vez). `false` si ya había uno.
    pub fn attach_tray(&self, icon: Arc<TrayIcon>) -> bool {
        self.tray.set(icon).is_ok()
    }

    #[must_use]
    pub const fn identity(&self) -> &OwnIdentity {
        &self.identity
    }
}

impl WindowsApi for Win32Api {
    fn processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        process::list(&self.identity)
    }

    fn top_level_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        window::top_level_windows()
    }

    fn foreground_window(&self, include_title: bool) -> Result<Option<ForegroundWindow>, PlatformError> {
        let Some(mut found) = window::foreground(include_title)? else {
            return Ok(None);
        };
        // El nombre del proceso se resuelve con la enumeración ya filtrada por sesión y usuario.
        found.process_name = self.processes()?.into_iter().find(|p| p.pid == found.pid).map(|p| p.name);
        Ok(Some(found))
    }

    fn launch(&self, target: &LaunchTarget) -> Result<Launch, PlatformError> {
        if let LaunchTarget::Executable(path) = target
            && !launch::is_trusted_executable(path)
        {
            return Err(launch::untrusted_error());
        }
        launch::launch(target)
    }

    fn request_close(&self, pids: &[u32]) -> Result<usize, PlatformError> {
        window::request_close(pids)
    }

    fn terminate(&self, pid: u32, created_at: Option<SystemTime>) -> Result<(), PlatformError> {
        process::terminate(pid, created_at)
    }

    fn session_state(&self) -> SessionState {
        session::state(self.idle_threshold)
    }

    fn resolve_app_path(&self, exe_name: &str) -> Option<PathBuf> {
        launch::resolve_app_path(exe_name)
    }

    fn expand_env(&self, text: &str) -> String {
        sys::expand_env(text)
    }

    fn is_trusted_executable(&self, path: &Path) -> bool {
        launch::is_trusted_executable(path)
    }

    fn open_sandboxed(&self, path: &Path) -> Result<Opened, PlatformError> {
        fs::open_sandboxed(path)
    }

    fn run_command(&self, spec: &CommandSpec, cancel: &CancelFlag) -> Result<CommandOutput, PlatformError> {
        commands::run(spec, cancel)
    }

    fn focus_window(&self, pids: &[u32]) -> Result<bool, PlatformError> {
        window::focus(pids)
    }

    fn foreground_pid(&self) -> Result<Option<u32>, PlatformError> {
        window::foreground_pid()
    }

    fn show_notification(&self, title: &str, body: &str) -> Result<(), PlatformError> {
        let Some(tray) = self.tray.get() else {
            return Err(PlatformError::new(PlatformErrorKind::Other, "no notification area is available"));
        };
        tray.balloon(title, body)
    }

    fn ui_window(&self, hwnd: isize, timeout: Duration) -> Result<UiWindow, PlatformError> {
        let window = self.uia.window(hwnd, timeout)?;
        Ok(UiWindow { hwnd: window.hwnd, pid: window.pid, name: window.name, class_name: window.class_name })
    }

    fn ui_find(
        &self,
        hwnd: isize,
        selector: &UiSelector,
        limit: usize,
        timeout: Duration,
    ) -> Result<Vec<UiElement>, PlatformError> {
        let found = self.uia.find_elements(hwnd, &selector_of(selector), limit, timeout)?;
        Ok(found
            .into_iter()
            .map(|element| UiElement {
                automation_id: element.automation_id,
                name: element.name,
                class_name: element.class_name,
                control_type: element.control_type,
                bounds: element.bounds,
                enabled: element.enabled,
                patterns: element.patterns,
                value: element.value,
                toggled: element.toggled,
                selected: element.selected,
            })
            .collect())
    }

    fn ui_invoke(&self, hwnd: isize, selector: &UiSelector, timeout: Duration) -> Result<String, PlatformError> {
        self.uia.invoke(hwnd, &selector_of(selector), timeout)
    }

    fn ui_set_value(
        &self,
        hwnd: isize,
        selector: &UiSelector,
        text: &str,
        timeout: Duration,
    ) -> Result<(), PlatformError> {
        self.uia.set_value(hwnd, &selector_of(selector), text, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_only_processes_of_this_session_and_user() {
        let api = Win32Api::new(Duration::from_secs(600)).expect("identity");
        let processes = api.processes().expect("processes");
        assert!(!processes.is_empty());
        let me = std::process::id();
        assert!(processes.iter().any(|p| p.pid == me), "el propio proceso debe aparecer");
        // Procesos de sistema de la sesión 0 nunca deben aparecer.
        assert!(!processes.iter().any(|p| p.name.eq_ignore_ascii_case("services.exe")));
    }

    #[test]
    fn reads_session_state_without_failing() {
        let api = Win32Api::new(Duration::from_secs(600)).expect("identity");
        let state = api.session_state();
        assert!(matches!(
            state,
            SessionState::Active | SessionState::Idle | SessionState::Locked | SessionState::Disconnected
        ));
    }

    #[test]
    fn expands_environment_variables() {
        let api = Win32Api::new(Duration::from_secs(600)).expect("identity");
        let expanded = api.expand_env(r"%SystemRoot%\System32\notepad.exe");
        assert!(expanded.to_lowercase().contains("windows"), "{expanded}");
        assert!(Path::new(&expanded).is_file(), "{expanded}");
    }

    #[test]
    fn rejects_executables_outside_trusted_locations() {
        let api = Win32Api::new(Duration::from_secs(600)).expect("identity");
        assert!(!api.is_trusted_executable(Path::new(r"C:\does\not\exist.exe")));
        let notepad = PathBuf::from(api.expand_env(r"%SystemRoot%\System32\notepad.exe"));
        assert!(api.is_trusted_executable(&notepad));

        // Un ejecutable con nombre de app conocida en una carpeta de descarga no es de fiar (WT-09).
        let temp = std::env::temp_dir().join("jarvis-impostor-chrome.exe");
        std::fs::copy(&notepad, &temp).expect("copy");
        assert!(!api.is_trusted_executable(&temp), "%TEMP% nunca es una ubicación de confianza");
        let _ = std::fs::remove_file(&temp);
    }
}
