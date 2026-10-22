//! Dobles de plataforma y transporte para probar el pipeline completo sin Windows ni MQTT.
//!
//! Cada binario de test usa una parte del doble; lo que no toca no es código muerto.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use chrono::Utc;
use jarvis_agent::health::Health;
use jarvis_agent::pipeline::{Agent, Publisher};
use jarvis_agent::tools::{ToolContext, build_registry};
use jarvis_policy::{Policy, PolicyState};
use jarvis_protocol::messages::{RequestedBy, TaskDispatch};
use jarvis_protocol::{Channel, Envelope};
use jarvis_state::Store;
use jarvis_tooling::platform::{
    CommandOutput, CommandSpec, DirEntryInfo, ForegroundWindow, Launch, LaunchTarget, LaunchedProcess, OpenEntry,
    Opened, PlatformError, PlatformErrorKind, ProcessInfo, SessionState, UiElement, UiSelector, UiWindow, WindowInfo,
    WindowsApi,
};
use jarvis_tooling::{Executor, ExecutorConfig};
use serde_json::Value;
use uuid::Uuid;

/// Qué ocurre cuando una tool lanza una aplicación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchBehaviour {
    /// El proceso y su ventana aparecen tras N observaciones.
    AppearsAfterPolls(u32),
    /// El proceso arranca y muere sin mostrar ventana (código de salida distinto de cero).
    ExitsWithCode(u32),
    /// El proceso vive pero nunca muestra ventana.
    NeverShowsWindow,
    /// El ejecutable no existe.
    NotFound,
}

#[derive(Debug)]
struct FakeState {
    processes: Vec<ProcessInfo>,
    windows: Vec<WindowInfo>,
    session: SessionState,
    launch: LaunchBehaviour,
    polls: u32,
    launched_pid: Option<u32>,
    launched_name: String,
    close_requests: usize,
    terminated: Vec<u32>,
    /// La app ignora `WM_CLOSE` (simula "¿guardar cambios?").
    ignores_close: bool,
    /// Sistema de archivos simulado (claves en minúsculas).
    files: std::collections::BTreeMap<String, FakeFile>,
    /// Rutas que el sandbox debe rechazar por ser reparse points.
    reparse_points: std::collections::BTreeSet<String>,
    /// Ruta real que devuelve el handle (simula un enlace intermedio ya resuelto).
    final_paths: std::collections::BTreeMap<String, String>,
    /// Comandos ejecutados y qué devuelve el siguiente.
    commands: Vec<CommandSpec>,
    command_output: CommandOutput,
    /// Foco: a quién se lo dio Windows y si lo permite.
    focused_pid: Option<u32>,
    focus_allowed: bool,
    notifications: Vec<(String, String)>,
    notifications_available: bool,
    /// Árbol de UI simulado por ventana, y acciones registradas.
    ui: Vec<UiElement>,
    ui_title: String,
    ui_invocations: Vec<(UiSelector, String)>,
    /// Patrón que responderá el próximo `ui_invoke` (o el error si está vacío).
    ui_invoke_pattern: Option<String>,
    /// Si las imágenes de los procesos se consideran de ubicación de confianza (WT-09).
    trusted: bool,
}

#[derive(Debug, Clone)]
struct FakeFile {
    is_dir: bool,
    content: Vec<u8>,
    hidden: bool,
}

/// Archivo o directorio abierto del sistema de archivos simulado.
#[derive(Debug)]
struct FakeOpen {
    content: Vec<u8>,
    children: Vec<DirEntryInfo>,
}

impl OpenEntry for FakeOpen {
    fn read(&self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, PlatformError> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(self.content.len());
        let end = start.saturating_add(max_bytes).min(self.content.len());
        Ok(self.content[start..end].to_vec())
    }

    fn list(&self, limit: usize) -> Result<Vec<DirEntryInfo>, PlatformError> {
        Ok(self.children.iter().take(limit).cloned().collect())
    }
}

/// Windows simulado: determinista y observable.
#[derive(Debug, Clone)]
pub struct FakeWindows {
    state: Arc<Mutex<FakeState>>,
}

impl FakeWindows {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                processes: Vec::new(),
                windows: Vec::new(),
                session: SessionState::Active,
                launch: LaunchBehaviour::AppearsAfterPolls(1),
                polls: 0,
                launched_pid: None,
                launched_name: "notepad.exe".to_owned(),
                close_requests: 0,
                terminated: Vec::new(),
                ignores_close: false,
                files: std::collections::BTreeMap::new(),
                reparse_points: std::collections::BTreeSet::new(),
                final_paths: std::collections::BTreeMap::new(),
                commands: Vec::new(),
                command_output: CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    truncated: false,
                    timed_out: false,
                    cancelled: false,
                },
                focused_pid: None,
                focus_allowed: true,
                notifications: Vec::new(),
                notifications_available: true,
                ui: Vec::new(),
                ui_title: "Ventana de prueba".to_owned(),
                ui_invocations: Vec::new(),
                ui_invoke_pattern: Some("Invoke".to_owned()),
                trusted: true,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_launch(&self, behaviour: LaunchBehaviour) {
        self.lock().launch = behaviour;
    }

    pub fn set_session(&self, session: SessionState) {
        self.lock().session = session;
    }

    pub fn set_ignores_close(&self, ignores: bool) {
        self.lock().ignores_close = ignores;
    }

    /// Añade un proceso ya en ejecución con su ventana.
    pub fn add_running(&self, pid: u32, name: &str, with_window: bool) {
        let mut state = self.lock();
        state.processes.push(process(pid, name));
        if with_window {
            state.windows.push(WindowInfo { handle: i64::from(pid) as isize, pid, class_name: "Fake".into() });
        }
    }

    /// Proceso con el nombre de una app conocida pero ejecutandose desde otra carpeta (WT-09).
    pub fn add_running_at(&self, pid: u32, name: &str, image_path: &str, with_window: bool) {
        let mut state = self.lock();
        let mut info = process(pid, name);
        info.image_path = Some(PathBuf::from(image_path));
        state.processes.push(info);
        if with_window {
            state.windows.push(WindowInfo { handle: i64::from(pid) as isize, pid, class_name: "Fake".into() });
        }
    }

    pub fn add_elevated(&self, pid: u32, name: &str) {
        let mut state = self.lock();
        let mut info = process(pid, name);
        info.elevated = Some(true);
        state.processes.push(info);
    }

    #[must_use]
    pub fn terminated(&self) -> Vec<u32> {
        self.lock().terminated.clone()
    }

    #[must_use]
    pub fn close_requests(&self) -> usize {
        self.lock().close_requests
    }
}

fn process(pid: u32, name: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: 1,
        name: name.to_owned(),
        image_path: Some(PathBuf::from(format!(r"C:\Windows\System32\{name}"))),
        created_at: Some(SystemTime::now()),
        elevated: Some(false),
    }
}

#[derive(Debug)]
struct FakeProcess {
    pid: u32,
    exit: Option<u32>,
}

impl LaunchedProcess for FakeProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn exit_code(&self) -> Option<u32> {
        self.exit
    }
}

impl FakeWindows {
    /// Añade un archivo al sistema de archivos simulado (crea los directorios que falten).
    pub fn add_file(&self, path: &str, content: &[u8]) {
        let mut state = self.lock();
        let mut parent = PathBuf::from(path);
        while parent.pop() && parent.parent().is_some() {
            state.files.entry(parent.to_string_lossy().to_lowercase()).or_insert(FakeFile {
                is_dir: true,
                content: Vec::new(),
                hidden: false,
            });
        }
        state.files.insert(path.to_lowercase(), FakeFile { is_dir: false, content: content.to_vec(), hidden: false });
    }

    /// Añade un archivo oculto.
    pub fn add_hidden_file(&self, path: &str, content: &[u8]) {
        self.add_file(path, content);
        if let Some(file) = self.lock().files.get_mut(&path.to_lowercase()) {
            file.hidden = true;
        }
    }

    pub fn add_dir(&self, path: &str) {
        self.lock().files.insert(path.to_lowercase(), FakeFile { is_dir: true, content: Vec::new(), hidden: false });
    }

    /// Marca una ruta como junction/symlink: el sandbox debe negarse a seguirla.
    pub fn add_reparse_point(&self, path: &str) {
        self.lock().reparse_points.insert(path.to_lowercase());
    }

    /// Hace que el handle de `path` diga que su ruta real es `real` (enlace intermedio).
    pub fn set_final_path(&self, path: &str, real: &str) {
        self.lock().final_paths.insert(path.to_lowercase(), real.to_owned());
    }

    pub fn set_command_output(&self, output: CommandOutput) {
        self.lock().command_output = output;
    }

    #[must_use]
    pub fn commands(&self) -> Vec<CommandSpec> {
        self.lock().commands.clone()
    }

    /// Simula que Windows deniega el cambio de foco (foreground lock).
    pub fn set_focus_allowed(&self, allowed: bool) {
        self.lock().focus_allowed = allowed;
    }

    #[must_use]
    pub fn focused_pid(&self) -> Option<u32> {
        self.lock().focused_pid
    }

    pub fn set_notifications_available(&self, available: bool) {
        self.lock().notifications_available = available;
    }

    #[must_use]
    pub fn notifications(&self) -> Vec<(String, String)> {
        self.lock().notifications.clone()
    }

    /// Añade un elemento al árbol de UI simulado.
    pub fn add_ui_element(&self, element: UiElement) {
        self.lock().ui.push(element);
    }

    /// Cambia el valor de un elemento ya existente (simula el efecto de una acción).
    pub fn set_ui_value(&self, automation_id: &str, value: Option<&str>) {
        let mut state = self.lock();
        if let Some(element) = state.ui.iter_mut().find(|e| e.automation_id == automation_id) {
            element.value = value.map(str::to_owned);
        }
    }

    /// Quita un elemento del árbol (simula un diálogo que se cierra).
    pub fn remove_ui_element(&self, automation_id: &str) {
        self.lock().ui.retain(|element| element.automation_id != automation_id);
    }

    /// Hace que `ui_invoke` falle por no haber patrón de acción.
    pub fn set_ui_invoke_pattern(&self, pattern: Option<&str>) {
        self.lock().ui_invoke_pattern = pattern.map(str::to_owned);
    }

    #[must_use]
    pub fn ui_invocations(&self) -> Vec<(UiSelector, String)> {
        self.lock().ui_invocations.clone()
    }

    pub fn set_ui_title(&self, title: &str) {
        self.lock().ui_title = title.to_owned();
    }

    /// Simula que los ejecutables están (o no) en una ubicación de confianza.
    pub fn set_trusted(&self, trusted: bool) {
        self.lock().trusted = trusted;
    }
}

/// Coincidencia del selector con un elemento del árbol simulado.
fn ui_matches(element: &UiElement, selector: &UiSelector) -> bool {
    selector.automation_id.as_ref().is_none_or(|id| *id == element.automation_id)
        && selector.name.as_ref().is_none_or(|name| *name == element.name)
        && selector.class_name.as_ref().is_none_or(|class| *class == element.class_name)
        && selector.control_type.as_ref().is_none_or(|control| *control == element.control_type)
}

impl WindowsApi for FakeWindows {
    fn processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        let mut state = self.lock();
        state.polls += 1;
        // Una app lanzada aparece tras el número de observaciones configurado.
        if let (LaunchBehaviour::AppearsAfterPolls(after), Some(pid)) = (state.launch, state.launched_pid)
            && state.polls > after
            && !state.processes.iter().any(|p| p.pid == pid)
        {
            let name = state.launched_name.clone();
            state.processes.push(process(pid, &name));
            state.windows.push(WindowInfo { handle: 1, pid, class_name: "Fake".into() });
        }
        if let (LaunchBehaviour::NeverShowsWindow, Some(pid)) = (state.launch, state.launched_pid)
            && !state.processes.iter().any(|p| p.pid == pid)
        {
            let name = state.launched_name.clone();
            state.processes.push(process(pid, &name));
        }
        Ok(state.processes.clone())
    }

    fn top_level_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        Ok(self.lock().windows.clone())
    }

    fn foreground_window(&self, include_title: bool) -> Result<Option<ForegroundWindow>, PlatformError> {
        let state = self.lock();
        let Some(window) = state.windows.first() else { return Ok(None) };
        let name = state.processes.iter().find(|p| p.pid == window.pid).map(|p| p.name.clone());
        Ok(Some(ForegroundWindow {
            pid: window.pid,
            process_name: name,
            title: include_title.then(|| "Jarvis - Visual Studio Code".to_owned()),
        }))
    }

    fn launch(&self, target: &LaunchTarget) -> Result<Launch, PlatformError> {
        let mut state = self.lock();
        if state.launch == LaunchBehaviour::NotFound {
            return Err(PlatformError::new(PlatformErrorKind::NotFound, "executable not found"));
        }
        // El proceso simulado toma el nombre del ejecutable lanzado (chrome.exe, notepad.exe…).
        state.launched_name = match target {
            LaunchTarget::Executable(path) => {
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "app.exe".to_owned())
            }
            LaunchTarget::PackagedApp(_) => "packaged.exe".to_owned(),
        };
        let pid = 4242;
        state.launched_pid = Some(pid);
        state.polls = 0;
        let exit = match state.launch {
            LaunchBehaviour::ExitsWithCode(code) => Some(code),
            _ => None,
        };
        Ok(Launch { process: Some(Box::new(FakeProcess { pid, exit })), launched_at: SystemTime::now() })
    }

    fn request_close(&self, pids: &[u32]) -> Result<usize, PlatformError> {
        let mut state = self.lock();
        state.close_requests += 1;
        if state.ignores_close {
            return Ok(pids.len());
        }
        state.processes.retain(|p| !pids.contains(&p.pid));
        state.windows.retain(|w| !pids.contains(&w.pid));
        Ok(pids.len())
    }

    fn terminate(&self, pid: u32, _created_at: Option<SystemTime>) -> Result<(), PlatformError> {
        let mut state = self.lock();
        state.terminated.push(pid);
        state.processes.retain(|p| p.pid != pid);
        state.windows.retain(|w| w.pid != pid);
        Ok(())
    }

    fn session_state(&self) -> SessionState {
        self.lock().session
    }

    fn resolve_app_path(&self, exe_name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!(r"C:\Windows\System32\{exe_name}")))
    }

    fn expand_env(&self, text: &str) -> String {
        text.replace("%SystemRoot%", r"C:\Windows")
    }

    fn is_trusted_executable(&self, _path: &Path) -> bool {
        self.lock().trusted
    }

    fn open_sandboxed(&self, path: &Path) -> Result<Opened, PlatformError> {
        let state = self.lock();
        let key = path.to_string_lossy().to_lowercase();
        if state.reparse_points.contains(&key) {
            return Err(PlatformError::new(PlatformErrorKind::AccessDenied, "reparse point"));
        }
        let file = state
            .files
            .get(&key)
            .ok_or_else(|| PlatformError::new(PlatformErrorKind::NotFound, "no such file"))?
            .clone();
        let final_path = state.final_paths.get(&key).map_or_else(|| path.to_path_buf(), PathBuf::from);

        let prefix = format!("{key}\\");
        let children: Vec<DirEntryInfo> = state
            .files
            .iter()
            .filter(|(child, _)| {
                child.starts_with(&prefix) && !child[prefix.len()..].contains(std::path::MAIN_SEPARATOR)
            })
            .map(|(child, entry)| DirEntryInfo {
                name: child[prefix.len()..].to_owned(),
                is_dir: entry.is_dir,
                size: entry.content.len() as u64,
                modified: Some(SystemTime::now()),
                hidden: entry.hidden,
            })
            .collect();

        Ok(Opened {
            final_path,
            is_dir: file.is_dir,
            size: file.content.len() as u64,
            modified: Some(SystemTime::now()),
            hidden: file.hidden,
            entry: Box::new(FakeOpen { content: file.content, children }),
        })
    }

    fn run_command(
        &self,
        spec: &CommandSpec,
        _cancel: &jarvis_tooling::platform::CancelFlag,
    ) -> Result<CommandOutput, PlatformError> {
        let mut state = self.lock();
        state.commands.push(spec.clone());
        Ok(state.command_output.clone())
    }

    fn focus_window(&self, pids: &[u32]) -> Result<bool, PlatformError> {
        let mut state = self.lock();
        if !state.focus_allowed {
            return Ok(false);
        }
        state.focused_pid = pids.first().copied();
        Ok(true)
    }

    fn foreground_pid(&self) -> Result<Option<u32>, PlatformError> {
        let state = self.lock();
        Ok(state.focused_pid.or_else(|| state.windows.first().map(|window| window.pid)))
    }

    fn show_notification(&self, title: &str, body: &str) -> Result<(), PlatformError> {
        let mut state = self.lock();
        if !state.notifications_available {
            return Err(PlatformError::new(PlatformErrorKind::Other, "notifications are disabled"));
        }
        state.notifications.push((title.to_owned(), body.to_owned()));
        Ok(())
    }

    fn ui_window(&self, hwnd: isize, _timeout: Duration) -> Result<UiWindow, PlatformError> {
        let state = self.lock();
        let window = state
            .windows
            .iter()
            .find(|window| window.handle == hwnd)
            .ok_or_else(|| PlatformError::new(PlatformErrorKind::NotFound, "no such window"))?;
        Ok(UiWindow { hwnd, pid: window.pid, name: state.ui_title.clone(), class_name: window.class_name.clone() })
    }

    fn ui_find(
        &self,
        _hwnd: isize,
        selector: &UiSelector,
        limit: usize,
        _timeout: Duration,
    ) -> Result<Vec<UiElement>, PlatformError> {
        let state = self.lock();
        Ok(state.ui.iter().filter(|element| ui_matches(element, selector)).take(limit).cloned().collect())
    }

    fn ui_invoke(&self, _hwnd: isize, selector: &UiSelector, _timeout: Duration) -> Result<String, PlatformError> {
        let mut state = self.lock();
        let matches = state.ui.iter().filter(|element| ui_matches(element, selector)).count();
        if matches == 0 {
            return Err(PlatformError::new(PlatformErrorKind::NotFound, "no element matches the selector"));
        }
        if matches > 1 {
            return Err(PlatformError::new(
                PlatformErrorKind::Other,
                format!("the selector matches {matches} elements: make it more specific"),
            ));
        }
        let Some(pattern) = state.ui_invoke_pattern.clone() else {
            return Err(PlatformError::new(PlatformErrorKind::Other, "the element exposes no action pattern"));
        };
        state.ui_invocations.push((selector.clone(), pattern.clone()));
        Ok(pattern)
    }

    fn ui_set_value(
        &self,
        _hwnd: isize,
        selector: &UiSelector,
        text: &str,
        _timeout: Duration,
    ) -> Result<(), PlatformError> {
        let mut state = self.lock();
        let Some(element) = state.ui.iter_mut().find(|element| ui_matches(element, selector)) else {
            return Err(PlatformError::new(PlatformErrorKind::NotFound, "no element matches the selector"));
        };
        if !element.patterns.iter().any(|pattern| pattern == "Value") {
            return Err(PlatformError::new(PlatformErrorKind::Other, "the element does not accept text"));
        }
        element.value = Some(text.to_owned());
        Ok(())
    }
}

/// Transporte simulado: registra lo publicado y puede simular estar sin conexión.
#[derive(Debug, Default)]
pub struct FakePublisher {
    online: Mutex<bool>,
    sent: Mutex<Vec<Envelope>>,
}

impl FakePublisher {
    #[must_use]
    pub fn new() -> Self {
        Self { online: Mutex::new(true), sent: Mutex::new(Vec::new()) }
    }

    pub fn set_online(&self, online: bool) {
        *self.online.lock().unwrap_or_else(PoisonError::into_inner) = online;
    }

    #[must_use]
    pub fn sent(&self) -> Vec<Envelope> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[must_use]
    pub fn kinds(&self) -> Vec<String> {
        self.sent().into_iter().map(|e| e.kind).collect()
    }

    /// Último mensaje de un tipo concreto.
    #[must_use]
    pub fn last(&self, kind: &str) -> Option<Envelope> {
        self.sent().into_iter().filter(|e| e.kind == kind).next_back()
    }

    #[must_use]
    pub fn count(&self, kind: &str) -> usize {
        self.sent().iter().filter(|e| e.kind == kind).count()
    }

    pub fn clear(&self) {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

#[async_trait::async_trait]
impl Publisher for FakePublisher {
    async fn publish(&self, _channel: Channel, payload: Vec<u8>) -> bool {
        if !*self.online.lock().unwrap_or_else(PoisonError::into_inner) {
            return false;
        }
        if let Ok(envelope) = Envelope::parse(&payload) {
            self.sent.lock().unwrap_or_else(PoisonError::into_inner).push(envelope);
        }
        true
    }
}

pub const OWNER: &str = "0192a6f0-0000-7aaa-8bbb-00000000000a";
pub const ESP32: &str = "0192a6f0-1111-7aaa-8bbb-000000000001";

/// Política de prueba: el owner puede abrir y cerrar (incluido `force`); el ESP32 solo observar y abrir.
#[must_use]
pub fn test_policy() -> Policy {
    let text = format!(
        r#"
version = 1
[[principals]]
id = "cloud:user:{OWNER}"
capabilities = ["windows.apps.read", "windows.window.read", "windows.window.title", "windows.apps.open", "windows.apps.close", "windows.apps.close.force"]
[[principals]]
id = "cloud:device:{ESP32}"
capabilities = ["windows.apps.read", "windows.apps.open"]
[[principals]]
id = "local:owner"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close"]
"#
    );
    Policy::parse(&text).expect("valid test policy")
}

/// Tools activas en el MVP.
pub const MVP_TOOLS: [&str; 5] = ["is_app_running", "list_running_apps", "get_active_window", "open_app", "close_app"];

pub struct Harness {
    pub agent: Arc<Agent>,
    pub publisher: Arc<FakePublisher>,
    pub windows: FakeWindows,
    pub device_id: Uuid,
    pub store_path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Harness {
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(test_policy())
    }

    #[must_use]
    pub fn with_policy(policy: Policy) -> Self {
        Self::with_policy_and_tools(policy, &MVP_TOOLS)
    }

    /// Harness con otras tools habilitadas (las de P7 no lo están por defecto).
    #[must_use]
    pub fn with_policy_and_tools(policy: Policy, enabled: &[&str]) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store_path = dir.path().join("agent.db");
        let store = Arc::new(Store::open(&store_path).expect("store"));
        Self::build(dir, store_path, store, policy, Arc::new(FakePublisher::new()), enabled)
    }

    /// Reabre la base existente (simula un reinicio del agente).
    #[must_use]
    pub fn restart(self) -> Self {
        let Self { store_path, _dir, windows: _, .. } = self;
        let store = Arc::new(Store::open(&store_path).expect("reopen"));
        Self::build(_dir, store_path, store, test_policy(), Arc::new(FakePublisher::new()), &MVP_TOOLS)
    }

    fn build(
        dir: tempfile::TempDir,
        store_path: PathBuf,
        store: Arc<Store>,
        policy: Policy,
        publisher: Arc<FakePublisher>,
        enabled: &[&str],
    ) -> Self {
        let windows = FakeWindows::new();
        let policy = Arc::new(policy);
        let ctx = ToolContext::new(Arc::new(windows.clone()), policy.clone());
        let enabled: Vec<String> = enabled.iter().map(|name| (*name).to_owned()).collect();
        let registry = Arc::new(build_registry(&ctx, &enabled).expect("registry"));
        let executor = Arc::new(Executor::new(ExecutorConfig { max_concurrent: 4 }));
        let state = Arc::new(PolicyState { policy: (*policy).clone(), valid: true });
        let device_id = Uuid::parse_str("0192a6f0-2222-7aaa-8bbb-000000000002").expect("uuid");
        let agent = Agent::new(
            device_id,
            store,
            registry,
            executor,
            state,
            Arc::new(Health::new()),
            publisher.clone() as Arc<dyn Publisher>,
        );
        Self { agent, publisher, windows, device_id, store_path, _dir: dir }
    }

    /// Construye y entrega un `task.dispatch` como lo haría el Cloud.
    pub async fn dispatch(&self, task_id: Uuid, task_type: &str, payload: Value, principal: &str) {
        self.dispatch_with(task_id, task_type, payload, principal, 300, None).await;
    }

    pub async fn dispatch_with(
        &self,
        task_id: Uuid,
        task_type: &str,
        payload: Value,
        principal: &str,
        expires_in_seconds: i64,
        max_duration_seconds: Option<u32>,
    ) {
        let envelope =
            self.dispatch_envelope(task_id, task_type, payload, principal, expires_in_seconds, max_duration_seconds);
        self.agent.handle_envelope(Channel::Tasks, envelope).await;
    }

    #[must_use]
    pub fn dispatch_envelope(
        &self,
        task_id: Uuid,
        task_type: &str,
        payload: Value,
        principal: &str,
        expires_in_seconds: i64,
        max_duration_seconds: Option<u32>,
    ) -> Envelope {
        let (kind, id) = if let Some(id) = principal.strip_prefix("cloud:user:") {
            ("User", id)
        } else {
            ("Device", principal.trim_start_matches("cloud:device:"))
        };
        let data = TaskDispatch {
            task_id,
            task_type: task_type.to_owned(),
            payload,
            attempt: 1,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in_seconds),
            max_duration_seconds,
            requested_by: RequestedBy { kind: kind.to_owned(), id: Uuid::parse_str(id).expect("principal uuid") },
            cancel_requested: false,
            signature: None,
        };
        Envelope::create("task.dispatch", self.device_id, &data, Some(Uuid::now_v7()), None).expect("envelope")
    }

    /// Espera a que se publique un mensaje del tipo indicado.
    /// Confirma un resultado como haría el Cloud (`task.update.ack`), que es lo que lo asienta.
    pub async fn ack(&self, task_id: Uuid) {
        let ack = Envelope::create(
            "task.update.ack",
            self.device_id,
            &serde_json::json!({"taskId": task_id, "status": "Completed"}),
            None,
            None,
        )
        .expect("envelope");
        self.agent.handle_envelope(Channel::Tasks, ack).await;
    }

    pub async fn wait_for(&self, kind: &str, timeout: Duration) -> Envelope {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(envelope) = self.publisher.last(kind) {
                return envelope;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {kind}: {:?}",
                self.publisher.sent().iter().map(|e| format!("{} {}", e.kind, e.data)).collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
