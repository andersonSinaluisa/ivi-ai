//! Abstracción de la plataforma Windows usada por las tools (implementada en `jarvis-windows`).
//!
//! Portable a propósito: las tools se prueban con dobles sin Windows real. Todas las funciones son
//! bloqueantes; las tools las llaman desde `spawn_blocking`.

use std::fmt;
use std::path::PathBuf;
use std::time::SystemTime;

/// Proceso de la **sesión y usuario del agente** (la implementación filtra el resto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    /// Nombre del ejecutable (`chrome.exe`).
    pub name: String,
    /// Ruta de imagen si es consultable.
    pub image_path: Option<PathBuf>,
    pub created_at: Option<SystemTime>,
    /// `Some(true)` si su token es elevado; `None` si no se pudo consultar (se trata como no controlable).
    pub elevated: Option<bool>,
}

/// Ventana top-level "real": visible, no *cloaked*, sin owner, no tool window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub handle: isize,
    pub pid: u32,
    pub class_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundWindow {
    pub pid: u32,
    pub process_name: Option<String>,
    /// Solo se lee si se solicita; nunca se registra en logs.
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Idle,
    Locked,
    Disconnected,
}

impl SessionState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Idle => "Idle",
            Self::Locked => "Locked",
            Self::Disconnected => "Disconnected",
        }
    }

    /// Con escritorio accesible (lanzar apps visibles, foco, UIA).
    #[must_use]
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::Active | Self::Idle)
    }
}

/// Cómo lanzar una app ya resuelta a partir del catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Ruta absoluta de un ejecutable Win32 (ya resuelta y comprobada).
    Executable(PathBuf),
    /// AUMID de una app empaquetada (MSIX/Store).
    PackagedApp(String),
}

/// Proceso lanzado; conserva el handle para poder leer su código de salida.
pub trait LaunchedProcess: Send + Sync + fmt::Debug {
    fn pid(&self) -> u32;
    /// `None` si sigue vivo; `Some(code)` si terminó.
    fn exit_code(&self) -> Option<u32>;
}

#[derive(Debug)]
pub struct Launch {
    pub process: Option<Box<dyn LaunchedProcess>>,
    pub launched_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformErrorKind {
    NotFound,
    AccessDenied,
    ElevationRequired,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct PlatformError {
    pub kind: PlatformErrorKind,
    pub message: String,
}

impl PlatformError {
    #[must_use]
    pub fn new(kind: PlatformErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

/// Entrada de un directorio (lo que ve `list_directory`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
}

/// Archivo o directorio ya abierto tras pasar el sandbox. Las lecturas se hacen **sobre el handle**,
/// no volviendo a abrir por ruta: así un cambio posterior de la ruta no puede colarse (TOCTOU).
pub trait OpenEntry: Send + Sync + fmt::Debug {
    /// Lee un trozo de archivo.
    ///
    /// # Errors
    /// Error del sistema de archivos.
    fn read(&self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, PlatformError>;

    /// Lista el directorio.
    ///
    /// # Errors
    /// Error del sistema de archivos.
    fn list(&self, limit: usize) -> Result<Vec<DirEntryInfo>, PlatformError>;
}

/// Resultado de abrir una ruta con las comprobaciones del sandbox.
#[derive(Debug)]
pub struct Opened {
    /// Ruta final real (resuelta desde el handle), que el llamante compara con las raíces permitidas.
    pub final_path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    pub entry: Box<dyn OpenEntry>,
}

/// Comando con nombre ya resuelto por la política (nunca viene del Cloud).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub capture_output: bool,
    pub max_output_bytes: usize,
    pub timeout: std::time::Duration,
}

/// Resultado de ejecutar un comando con nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub timed_out: bool,
    /// El comando se detuvo porque alguien canceló la tarea, no porque terminara.
    pub cancelled: bool,
}

/// Bandera que el ejecutor levanta para abortar un comando en marcha.
///
/// Un comando se ejecuta en un hilo bloqueante: sin esta señal, un `task.cancel` no podría
/// detener al proceso hijo y la cancelación sería solo nominal.
pub type CancelFlag = std::sync::atomic::AtomicBool;

/// Selector semántico de un elemento de UI (ADR-012: nunca coordenadas).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiSelector {
    pub automation_id: Option<String>,
    pub name: Option<String>,
    pub class_name: Option<String>,
    pub control_type: Option<String>,
}

/// Elemento de UI con sus propiedades ya leídas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiElement {
    pub automation_id: String,
    /// Puede contener datos personales: se trata como sensible.
    pub name: String,
    pub class_name: String,
    pub control_type: String,
    pub bounds: (i32, i32, i32, i32),
    pub enabled: bool,
    pub patterns: Vec<String>,
    pub value: Option<String>,
    pub toggled: Option<bool>,
    pub selected: Option<bool>,
}

/// Ventana de nivel superior vista por UI Automation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiWindow {
    pub hwnd: isize,
    pub pid: u32,
    /// Título; se redacta según la política de privacidad antes de publicarlo.
    pub name: String,
    pub class_name: String,
}

pub trait WindowsApi: Send + Sync {
    /// Procesos de la sesión y del usuario del agente.
    ///
    /// # Errors
    /// Error del sistema al enumerar.
    fn processes(&self) -> Result<Vec<ProcessInfo>, PlatformError>;

    /// Ventanas top-level reales de la sesión.
    ///
    /// # Errors
    /// Error del sistema al enumerar.
    fn top_level_windows(&self) -> Result<Vec<WindowInfo>, PlatformError>;

    /// Ventana en primer plano; `None` si no hay (sesión bloqueada, Secure Desktop).
    ///
    /// # Errors
    /// Error del sistema.
    fn foreground_window(&self, include_title: bool) -> Result<Option<ForegroundWindow>, PlatformError>;

    /// Lanza una app sin argumentos.
    ///
    /// # Errors
    /// `NotFound`, `AccessDenied`, `ElevationRequired` u `Other`.
    fn launch(&self, target: &LaunchTarget) -> Result<Launch, PlatformError>;

    /// Envía `WM_CLOSE` a las ventanas top-level de esos procesos. Devuelve cuántas ventanas se señalaron.
    ///
    /// # Errors
    /// Error del sistema.
    fn request_close(&self, pids: &[u32]) -> Result<usize, PlatformError>;

    /// Termina un proceso, comprobando su hora de creación para no matar un PID reutilizado.
    ///
    /// # Errors
    /// `AccessDenied` (p. ej. elevado) u `Other`.
    fn terminate(&self, pid: u32, created_at: Option<SystemTime>) -> Result<(), PlatformError>;

    fn session_state(&self) -> SessionState;

    /// Resuelve un nombre de `App Paths` (HKCU y luego HKLM) a una ruta absoluta existente.
    fn resolve_app_path(&self, exe_name: &str) -> Option<PathBuf>;

    /// Expande `%VARIABLES%` de entorno.
    fn expand_env(&self, text: &str) -> String;

    /// `true` si la ruta es un archivo existente en una carpeta que otros usuarios no pueden modificar.
    fn is_trusted_executable(&self, path: &std::path::Path) -> bool;

    /// Abre una ruta aplicando las comprobaciones del sandbox: sin seguir reparse points, solo en
    /// volúmenes fijos, y devolviendo la ruta final real para que el llamante valide las raíces.
    ///
    /// # Errors
    /// `NotFound`, `AccessDenied` (reparse point, unidad de red) u `Other`.
    fn open_sandboxed(&self, path: &std::path::Path) -> Result<Opened, PlatformError>;

    /// Ejecuta un comando del catálogo local dentro de un Job Object (mata también a sus hijos).
    ///
    /// # Errors
    /// `NotFound` si el ejecutable no existe; `AccessDenied` si requiere elevación.
    fn run_command(&self, spec: &CommandSpec, cancel: &CancelFlag) -> Result<CommandOutput, PlatformError>;

    /// Trae al primer plano la ventana principal de alguno de esos procesos.
    ///
    /// # Errors
    /// Error del sistema.
    fn focus_window(&self, pids: &[u32]) -> Result<bool, PlatformError>;

    /// PID de la ventana en primer plano (para verificar `focus_window`).
    ///
    /// # Errors
    /// Error del sistema.
    fn foreground_pid(&self) -> Result<Option<u32>, PlatformError>;

    /// Muestra una notificación al usuario. Sin botones de acción (WT-15).
    ///
    /// # Errors
    /// Si no hay área de notificación disponible o el usuario las tiene desactivadas.
    fn show_notification(&self, title: &str, body: &str) -> Result<(), PlatformError>;

    /// Datos de una ventana según UI Automation (incluye su título).
    ///
    /// # Errors
    /// `NotFound` si la ventana ya no existe; `Other` si UIA no responde a tiempo.
    fn ui_window(&self, hwnd: isize, timeout: std::time::Duration) -> Result<UiWindow, PlatformError>;

    /// Elementos de esa ventana que cumplen el selector.
    ///
    /// # Errors
    /// `NotFound` si la ventana desapareció; `Other` si UIA no responde.
    fn ui_find(
        &self,
        hwnd: isize,
        selector: &UiSelector,
        limit: usize,
        timeout: std::time::Duration,
    ) -> Result<Vec<UiElement>, PlatformError>;

    /// Actúa sobre el elemento con su patrón UIA. Devuelve el patrón usado.
    ///
    /// # Errors
    /// `NotFound` si no hay elemento; `Other` si hay varios, si no expone patrón o si UIA no responde.
    fn ui_invoke(
        &self,
        hwnd: isize,
        selector: &UiSelector,
        timeout: std::time::Duration,
    ) -> Result<String, PlatformError>;

    /// Escribe texto con `ValuePattern.SetValue` (nunca simulando teclado).
    ///
    /// # Errors
    /// `AccessDenied` si el elemento es de solo lectura; `Other` si no admite el patrón.
    fn ui_set_value(
        &self,
        hwnd: isize,
        selector: &UiSelector,
        text: &str,
        timeout: std::time::Duration,
    ) -> Result<(), PlatformError>;
}
