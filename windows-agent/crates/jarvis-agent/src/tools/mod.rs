//! Tools del MVP (docs/03-tools/process-tools.md).
//!
//! Las tools no conocen MQTT: reciben input JSON validado y devuelven `ToolResult` a través del
//! executor. Sirven igual a una tarea del Cloud, a la CLI local y al futuro Agent Core (ADR-014).

pub mod apps;
pub mod commands;
pub mod files;
pub mod privacy;
pub mod ui;
pub mod ui_automation;

use std::sync::Arc;

use jarvis_policy::Policy;
use jarvis_tooling::platform::{ProcessInfo, WindowsApi};
use jarvis_tooling::{ToolError, ToolRegistry, registry::Alias};

pub use apps::{CloseApp, GetActiveWindow, IsAppRunning, ListRunningApps, OpenApp};
pub use commands::RunCommand;
pub use files::{ListDirectory, ReadFile};
pub use ui::{FocusWindow, ShowNotification};
pub use ui_automation::{FindElement, FindWindow, GetText, InvokeElement, SetText};

/// Dependencias compartidas por las tools.
///
/// La política se lee en cada uso (no se copia al construir la tool) para que `policy reload`
/// afecte también al catálogo de apps y a las reglas de privacidad.
pub struct ToolContext {
    pub api: Arc<dyn WindowsApi>,
    policy: std::sync::RwLock<Arc<Policy>>,
}

impl ToolContext {
    #[must_use]
    pub fn new(api: Arc<dyn WindowsApi>, policy: Arc<Policy>) -> Arc<Self> {
        Arc::new(Self { api, policy: std::sync::RwLock::new(policy) })
    }

    /// Política vigente en este momento.
    #[must_use]
    pub fn policy(&self) -> Arc<Policy> {
        self.policy.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Sustituye la política (la llama `policy reload`).
    pub fn set_policy(&self, policy: Arc<Policy>) {
        *self.policy.write().unwrap_or_else(std::sync::PoisonError::into_inner) = policy;
    }

    /// Ejecuta una llamada bloqueante de Windows fuera del runtime async.
    pub async fn blocking<T, F>(&self, f: F) -> Result<T, ToolError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn WindowsApi) -> Result<T, jarvis_tooling::platform::PlatformError> + Send + 'static,
    {
        let api = self.api.clone();
        tokio::task::spawn_blocking(move || f(api.as_ref()))
            .await
            .map_err(|_| ToolError::new(jarvis_tooling::ErrorCode::SystemError, "windows call panicked"))?
            .map_err(platform_error)
    }
}

/// Traduce un error de plataforma al catálogo de errores del contrato.
#[must_use]
pub fn platform_error(error: jarvis_tooling::platform::PlatformError) -> ToolError {
    use jarvis_tooling::ErrorCode;
    use jarvis_tooling::platform::PlatformErrorKind as Kind;
    let code = match error.kind {
        Kind::NotFound => ErrorCode::AppNotFound,
        Kind::AccessDenied | Kind::ElevationRequired => ErrorCode::AccessDenied,
        Kind::Other => ErrorCode::ExecutionFailed,
    };
    ToolError::new(code, error.message)
}

/// Procesos que pertenecen a una entrada del catálogo.
///
/// Anti-suplantación (WT-09): un ejecutable con el mismo nombre en una carpeta escribible por el
/// usuario (`%TEMP%`, Descargas…) **no** cuenta como la aplicación del catálogo.
///
/// No se exige que la ruta sea idéntica a la del catálogo porque Windows usa lanzadores: el
/// `notepad.exe` de System32 abre el Notepad empaquetado bajo `WindowsApps`, y un navegador puede
/// ejecutarse desde una carpeta de versión distinta a la registrada. El criterio es: mismo nombre
/// **y** (ruta exacta del catálogo **o** imagen en una ubicación de confianza).
#[must_use]
pub fn processes_of(
    processes: &[ProcessInfo],
    process_names: &[String],
    expected_path: Option<&std::path::Path>,
    is_trusted: &dyn Fn(&std::path::Path) -> bool,
) -> Vec<ProcessInfo> {
    processes
        .iter()
        .filter(|p| {
            if !process_names.iter().any(|n| n.eq_ignore_ascii_case(&p.name)) {
                return false;
            }
            match p.image_path.as_deref() {
                Some(actual) => {
                    let same_as_catalog = expected_path.is_some_and(|expected| {
                        actual.to_string_lossy().eq_ignore_ascii_case(&expected.to_string_lossy())
                    });
                    same_as_catalog || is_trusted(actual)
                }
                // Imagen no legible (proceso protegido o elevado): se conserva para que las tools
                // puedan informarlo en lugar de ignorarlo en silencio.
                None => true,
            }
        })
        .cloned()
        .collect()
}

/// Registro con las tools del MVP y de P7, más el alias `OPEN_CHROME` (ADR-015).
///
/// Estar registrada no es estar activa: `enabled` (config.toml) decide cuáles se exponen.
///
/// # Errors
/// Si un descriptor es inválido o `enabled` menciona una tool inexistente: el agente no arranca.
pub fn build_registry(ctx: &Arc<ToolContext>, enabled: &[String]) -> Result<ToolRegistry, String> {
    ToolRegistry::builder()
        .tool(Arc::new(IsAppRunning::new(ctx.clone())))
        .tool(Arc::new(ListRunningApps::new(ctx.clone())))
        .tool(Arc::new(GetActiveWindow::new(ctx.clone())))
        .tool(Arc::new(OpenApp::new(ctx.clone())))
        .tool(Arc::new(CloseApp::new(ctx.clone())))
        // P7: deshabilitadas salvo que el owner las active en config.toml y configure raíces/comandos.
        .tool(Arc::new(ListDirectory::new(ctx.clone())))
        .tool(Arc::new(ReadFile::new(ctx.clone())))
        .tool(Arc::new(RunCommand::new(ctx.clone())))
        .tool(Arc::new(FocusWindow::new(ctx.clone())))
        .tool(Arc::new(ShowNotification::new(ctx.clone())))
        // P9: UI Automation, también deshabilitadas por defecto.
        .tool(Arc::new(FindWindow::new(ctx.clone())))
        .tool(Arc::new(FindElement::new(ctx.clone())))
        .tool(Arc::new(GetText::new(ctx.clone())))
        .tool(Arc::new(InvokeElement::new(ctx.clone())))
        .tool(Arc::new(SetText::new(ctx.clone())))
        .alias(Alias {
            task_type: "OPEN_CHROME",
            tool: "open_app",
            // El `url` opcional del contrato del Cloud se ignora: open_app no acepta URLs (C3).
            transform: |_payload| serde_json::json!({ "appId": "chrome" }),
        })
        .build(enabled)
}
