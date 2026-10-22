//! Tools de aplicaciones del MVP: observar, abrir y cerrar (docs/03-tools/process-tools.md).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use jarvis_policy::apps::{AppEntry, LaunchSpec};
use jarvis_tooling::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationSpec};
use jarvis_tooling::platform::{LaunchTarget, LaunchedProcess, ProcessInfo};
use jarvis_tooling::{
    Check, ErrorCode, ExecContext, ExecutionOutcome, ResourceLock, RiskLevel, Tool, ToolDescriptor, ToolError,
    VerificationMode, VerificationReport,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{ToolContext, processes_of};

const MAX_OUTPUT: usize = 24 * 1024;

#[allow(clippy::too_many_arguments)] // Construye el descriptor completo del contrato de Tool.
pub(super) fn descriptor(
    name: &'static str,
    description: &'static str,
    risk: RiskLevel,
    permissions: Vec<&'static str>,
    input_schema: Value,
    output_schema: Value,
    timeouts: Timeouts,
    cancellation: CancellationMode,
    verification: VerificationSpec,
    sensitivity: Sensitivity,
    requires_interactive_session: bool,
) -> ToolDescriptor {
    ToolDescriptor {
        name,
        version: "1.0.0",
        description,
        risk_level: risk,
        required_permissions: permissions,
        input_schema,
        output_schema,
        timeouts,
        cancellation,
        verification,
        sensitivity,
        max_output_bytes: MAX_OUTPUT,
        requires_interactive_session,
        idempotent_effect: true,
    }
}

fn app_id_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["appId"],
        "properties": { "appId": { "type": "string", "minLength": 1, "maxLength": 64 } }
    })
}

/// Resuelve el `appId` contra el catálogo local y devuelve el input normalizado.
pub(super) fn resolve_app(ctx: &ToolContext, query: &str) -> Result<(String, AppEntry), ToolError> {
    match ctx.policy().apps.resolve(query) {
        Ok((id, entry)) => Ok((id.to_owned(), entry.clone())),
        Err(jarvis_policy::ResolveError::Invalid) => {
            Err(ToolError::invalid("appId must be a catalog id, display name or alias (never a path)"))
        }
        Err(jarvis_policy::ResolveError::NotFound(name)) => {
            Err(ToolError::new(ErrorCode::AppNotFound, format!("application '{name}' is not in the local catalog")))
        }
    }
}

pub(super) fn entry_of(ctx: &ToolContext, input: &Value) -> Result<(String, AppEntry), ToolError> {
    let app_id = input.get("appId").and_then(Value::as_str).ok_or_else(|| ToolError::invalid("missing appId"))?;
    resolve_app(ctx, app_id)
}

/// Ruta esperada del ejecutable, cuando el catálogo la conoce (comprobación anti-suplantación).
pub(super) fn expected_path(ctx: &ToolContext, entry: &AppEntry) -> Option<PathBuf> {
    match &entry.launch {
        LaunchSpec::AppPaths { name } => ctx.api.resolve_app_path(name),
        LaunchSpec::Path { path } => Some(PathBuf::from(ctx.api.expand_env(path))),
        LaunchSpec::Aumid { .. } => None,
    }
}

pub(super) async fn app_processes(
    ctx: &ToolContext,
    entry: &AppEntry,
    path: Option<PathBuf>,
) -> Result<Vec<ProcessInfo>, ToolError> {
    let names = entry.process_names.clone();
    let all = ctx.blocking(move |api| api.processes()).await?;
    let api = ctx.api.clone();
    Ok(processes_of(&all, &names, path.as_deref(), &|image| api.is_trusted_executable(image)))
}

// ---- is_app_running ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AppIdInput {
    app_id: String,
}

pub struct IsAppRunning {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl IsAppRunning {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "is_app_running",
                "Checks whether an application from the local catalog is running",
                RiskLevel::R0,
                vec!["windows.apps.read"],
                app_id_schema(),
                json!({"type": "object", "properties": {"appId": {"type": "string"}, "running": {"type": "boolean"}, "processCount": {"type": "integer"}}}),
                Timeouts { default: Duration::from_secs(2), max: Duration::from_secs(5) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                Sensitivity::None,
                false,
            ),
        }
    }
}

#[async_trait]
impl Tool for IsAppRunning {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: AppIdInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        let (id, _) = resolve_app(&self.ctx, &parsed.app_id)?;
        Ok(json!({ "appId": id }))
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (app_id, entry) = entry_of(&self.ctx, input)?;
        let path = expected_path(&self.ctx, &entry);
        let processes = app_processes(&self.ctx, &entry, path).await?;
        let listed: Vec<Value> = processes.iter().take(50).map(|p| json!({ "pid": p.pid, "name": p.name })).collect();
        Ok(ExecutionOutcome {
            output: json!({
                "appId": app_id,
                "running": !processes.is_empty(),
                "processCount": processes.len(),
                "processes": listed,
            }),
            ..Default::default()
        })
    }
}

// ---- list_running_apps ------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListInput {
    #[serde(default)]
    include_uncatalogued: bool,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_limit() -> usize {
    100
}

pub struct ListRunningApps {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl ListRunningApps {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "list_running_apps",
                "Lists user applications that are currently running",
                RiskLevel::R0,
                vec!["windows.apps.read"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "includeUncatalogued": {"type": "boolean", "default": false},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                    }
                }),
                json!({"type": "object", "properties": {"apps": {"type": "array"}, "truncated": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(3), max: Duration::from_secs(10) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                Sensitivity::None,
                false,
            ),
        }
    }
}

#[async_trait]
impl Tool for ListRunningApps {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: ListInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if !(1..=200).contains(&parsed.limit) {
            return Err(ToolError::invalid("limit must be in 1..=200"));
        }
        Ok(json!({ "includeUncatalogued": parsed.include_uncatalogued, "limit": parsed.limit }))
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let include_uncatalogued = input.get("includeUncatalogued").and_then(Value::as_bool).unwrap_or(false);
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;

        let processes = self.ctx.blocking(move |api| api.processes()).await?;
        let windows = self.ctx.blocking(move |api| api.top_level_windows()).await?;
        let with_window: std::collections::HashSet<u32> = windows.iter().map(|w| w.pid).collect();

        // Agrupa por aplicación del catálogo; el resto (si se pide) por nombre de ejecutable, y solo
        // si tiene ventana visible: así no se expone la lista de servicios y procesos internos.
        let mut grouped: BTreeMap<String, (Option<String>, usize, bool)> = BTreeMap::new();
        for process in &processes {
            let app_id = self.ctx.policy().apps.app_for_process(&process.name).map(str::to_owned);
            let visible = with_window.contains(&process.pid);
            if app_id.is_none() && (!include_uncatalogued || !visible) {
                continue;
            }
            let key = app_id.clone().unwrap_or_else(|| process.name.to_lowercase());
            let entry = grouped.entry(key).or_insert((app_id, 0, false));
            entry.1 += 1;
            entry.2 |= visible;
        }

        let total = grouped.len();
        let apps: Vec<Value> = grouped
            .into_iter()
            .take(limit)
            .map(|(key, (app_id, count, visible))| {
                json!({
                    "appId": app_id,
                    "processName": key,
                    "processCount": count,
                    "hasVisibleWindow": visible,
                })
            })
            .collect();
        Ok(ExecutionOutcome { output: json!({ "apps": apps, "truncated": total > limit }), ..Default::default() })
    }
}

// ---- get_active_window ------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ActiveWindowInput {
    #[serde(default)]
    include_title: bool,
}

pub struct GetActiveWindow {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl GetActiveWindow {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "get_active_window",
                "Returns the foreground window (process and, if allowed, a redacted title)",
                RiskLevel::R1,
                vec!["windows.window.read"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "includeTitle": {"type": "boolean", "default": false} }
                }),
                json!({"type": "object", "properties": {"process": {"type": "string"}, "pid": {"type": "integer"}, "title": {"type": "string"}, "titleRedacted": {"type": "boolean"}, "sessionState": {"type": "string"}}}),
                Timeouts { default: Duration::from_secs(1), max: Duration::from_secs(3) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                // El título puede contener datos personales: se redacta en la BD tras *settled*.
                Sensitivity::Output,
                false,
            ),
        }
    }
}

#[async_trait]
impl Tool for GetActiveWindow {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: ActiveWindowInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        Ok(json!({ "includeTitle": parsed.include_title }))
    }

    /// Leer el título exige además la capability `windows.window.title`.
    fn required_permissions(&self, input: &Value) -> Vec<String> {
        let mut permissions = vec!["windows.window.read".to_owned()];
        if input.get("includeTitle").and_then(Value::as_bool).unwrap_or(false) {
            permissions.push("windows.window.title".to_owned());
        }
        permissions
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let include_title = input.get("includeTitle").and_then(Value::as_bool).unwrap_or(false);
        let session = self.ctx.blocking(|api| Ok(api.session_state())).await?;
        if !session.is_interactive() {
            // Sesión bloqueada: no es un error, simplemente no hay ventana observable.
            return Ok(ExecutionOutcome { output: json!({ "sessionState": session.as_str() }), ..Default::default() });
        }
        let found = self.ctx.blocking(move |api| api.foreground_window(include_title)).await?;
        let Some(window) = found else {
            return Ok(ExecutionOutcome { output: json!({ "sessionState": session.as_str() }), ..Default::default() });
        };
        let policy = self.ctx.policy();
        let (title, redacted) =
            super::privacy::apply(&policy.privacy, window.process_name.as_deref(), window.title.as_deref());
        // El título es dato sensible: se envuelve para que no pueda acabar en un log por descuido.
        let title = title.map(crate::logging::Sensitive::new);
        let app_id =
            window.process_name.as_deref().and_then(|name| policy.apps.app_for_process(name)).map(str::to_owned);
        Ok(ExecutionOutcome {
            output: json!({
                "appId": app_id,
                "process": window.process_name,
                "pid": window.pid,
                "title": title.as_ref().map(crate::logging::Sensitive::expose),
                "titleRedacted": redacted,
                "sessionState": session.as_str(),
            }),
            ..Default::default()
        })
    }
}

// ---- open_app ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OpenInput {
    app_id: String,
    #[serde(default = "noop_mode")]
    if_running: String,
}

fn noop_mode() -> String {
    "noop".to_owned()
}

pub struct OpenApp {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
    /// Procesos lanzados pendientes de verificar (su handle permite leer el código de salida).
    launched: Mutex<BTreeMap<Uuid, Box<dyn LaunchedProcess>>>,
}

impl OpenApp {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "open_app",
                "Opens an application from the local catalog and verifies that it is really running",
                RiskLevel::R2,
                vec!["windows.apps.open"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["appId"],
                    "properties": {
                        "appId": {"type": "string", "minLength": 1, "maxLength": 64},
                        "ifRunning": {"enum": ["noop", "new_instance"], "default": "noop"}
                    }
                }),
                json!({"type": "object", "properties": {"appId": {"type": "string"}, "processId": {"type": "integer"}, "alreadyRunning": {"type": "boolean"}, "action": {"type": "string"}}}),
                Timeouts { default: Duration::from_secs(15), max: Duration::from_secs(60) },
                CancellationMode::Cooperative,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "process_and_window",
                    timeout: Duration::from_secs(10),
                    poll_interval: Duration::from_millis(250),
                },
                Sensitivity::None,
                // Lanzar una app con la sesión bloqueada no es observable para el usuario.
                true,
            ),
            launched: Mutex::new(BTreeMap::new()),
        }
    }

    fn target(&self, entry: &AppEntry) -> Result<LaunchTarget, ToolError> {
        match &entry.launch {
            LaunchSpec::AppPaths { name } => self
                .ctx
                .api
                .resolve_app_path(name)
                .map(LaunchTarget::Executable)
                .ok_or_else(|| ToolError::new(ErrorCode::AppNotFound, format!("'{name}' is not installed"))),
            LaunchSpec::Path { path } => {
                let expanded = PathBuf::from(self.ctx.api.expand_env(path));
                if expanded.is_file() {
                    Ok(LaunchTarget::Executable(expanded))
                } else {
                    Err(ToolError::new(ErrorCode::AppNotFound, "the configured executable does not exist"))
                }
            }
            LaunchSpec::Aumid { id } => Ok(LaunchTarget::PackagedApp(id.clone())),
        }
    }
}

#[async_trait]
impl Tool for OpenApp {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: OpenInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if !matches!(parsed.if_running.as_str(), "noop" | "new_instance") {
            // `focus` llegará con focus_window (fase P7): no se acepta algo que no se puede verificar.
            return Err(ToolError::invalid("ifRunning must be 'noop' or 'new_instance'"));
        }
        let (id, _) = resolve_app(&self.ctx, &parsed.app_id)?;
        Ok(json!({ "appId": id, "ifRunning": parsed.if_running }))
    }

    fn resources(&self, input: &Value) -> Vec<ResourceLock> {
        input.get("appId").and_then(Value::as_str).map(|id| vec![ResourceLock::app(id)]).unwrap_or_default()
    }

    async fn execute(&self, input: &Value, ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (app_id, entry) = entry_of(&self.ctx, input)?;
        let if_running = input.get("ifRunning").and_then(Value::as_str).unwrap_or("noop");

        let session = self.ctx.blocking(|api| Ok(api.session_state())).await?;
        if !session.is_interactive() {
            return Err(ToolError::new(
                ErrorCode::SessionUnavailable,
                format!("the session is {} and the application would not be visible", session.as_str()),
            ));
        }

        let path = expected_path(&self.ctx, &entry);
        let existing = app_processes(&self.ctx, &entry, path.clone()).await?;
        let already_running = !existing.is_empty();
        if already_running && if_running == "noop" {
            return Ok(ExecutionOutcome {
                output: json!({
                    "appId": app_id,
                    "processId": existing.first().map(|p| p.pid),
                    "alreadyRunning": true,
                    "action": "none",
                }),
                details: Some(json!({ "method": "already_running" })),
                evidence: json!({ "mode": "already_running" }),
            });
        }

        let target = self.target(&entry)?;
        let launch = self.ctx.blocking(move |api| api.launch(&target)).await?;
        let launched_pid = launch.process.as_ref().map(|p| p.pid()).filter(|pid| *pid != 0);
        let launched_at =
            launch.launched_at.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
        if let Some(process) = launch.process {
            self.launched.lock().unwrap_or_else(PoisonError::into_inner).insert(ctx.execution_id, process);
        }

        Ok(ExecutionOutcome {
            output: json!({
                "appId": app_id,
                "processId": launched_pid,
                "alreadyRunning": already_running,
                "action": "launched",
            }),
            details: Some(json!({ "method": "shell_execute" })),
            evidence: json!({ "mode": "launched", "launchedPid": launched_pid, "launchedAtMs": launched_at }),
        })
    }

    /// `process_and_window`: proceso vivo **y** ventana top-level visible.
    async fn verify(&self, input: &Value, outcome: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
        let Ok((_, entry)) = entry_of(&self.ctx, input) else {
            return VerificationReport::inconclusive(Vec::new(), "the catalog entry disappeared");
        };
        let launched = self.launched.lock().unwrap_or_else(PoisonError::into_inner).remove(&ctx.execution_id);
        let launched_pid = outcome.evidence.get("launchedPid").and_then(Value::as_u64).map(|p| p as u32);
        let launched_at_ms = outcome.evidence.get("launchedAtMs").and_then(Value::as_u64);
        let path = expected_path(&self.ctx, &entry);
        let budget = self
            .descriptor
            .verification
            .timeout
            .min(entry.verify_timeout_ms.map_or(Duration::from_secs(10), Duration::from_millis));
        let deadline = tokio::time::Instant::now() + budget;

        let mut last_pids: Vec<u32>;
        loop {
            let processes = match app_processes(&self.ctx, &entry, path.clone()).await {
                Ok(processes) => processes,
                Err(error) => return VerificationReport::inconclusive(Vec::new(), error.message),
            };
            // Vale el PID lanzado o cualquier proceso de la app creado tras el lanzamiento
            // (apps de instancia única y launchers que ceden el testigo a otro proceso).
            let relevant: Vec<&ProcessInfo> = processes
                .iter()
                .filter(|p| {
                    launched_pid == Some(p.pid)
                        || launched_at_ms.is_none_or(|launched| created_ms(p) + 2_000 >= launched)
                })
                .collect();
            last_pids = relevant.iter().map(|p| p.pid).collect();

            if !relevant.is_empty() {
                let windows = match self.ctx.blocking(move |api| api.top_level_windows()).await {
                    Ok(windows) => windows,
                    Err(error) => return VerificationReport::inconclusive(Vec::new(), error.message),
                };
                if let Some(window) = windows.iter().find(|w| last_pids.contains(&w.pid)) {
                    let checks = vec![
                        Check::new("process_running", true, Some(json!({ "processCount": relevant.len() }))),
                        Check::new("top_level_window_visible", true, Some(json!({ "pid": window.pid }))),
                    ];
                    return VerificationReport::passed(checks).with_output_patch(json!({ "processId": window.pid }));
                }
            }

            // El proceso lanzado murió sin llegar a mostrar ventana: fallo con su código de salida.
            if relevant.is_empty()
                && let Some(code) = launched.as_ref().and_then(|p| p.exit_code())
            {
                let checks = vec![Check::new("process_running", false, Some(json!({ "exitCode": code })))];
                return VerificationReport::failed(
                    checks,
                    format!("the process exited with code {code} before showing a window"),
                );
            }

            if tokio::time::Instant::now() >= deadline {
                let checks = vec![
                    Check::new(
                        "process_running",
                        !last_pids.is_empty(),
                        Some(json!({ "processCount": last_pids.len() })),
                    ),
                    Check::new("top_level_window_visible", false, None),
                ];
                return VerificationReport::failed(checks, "no visible window appeared within the verification budget");
            }
            if ctx.sleep(self.descriptor.verification.poll_interval).await.is_err() {
                return VerificationReport::inconclusive(Vec::new(), "verification cancelled");
            }
        }
    }
}

fn created_ms(process: &ProcessInfo) -> u64 {
    process
        .created_at
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}

// ---- close_app --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CloseInput {
    app_id: String,
    #[serde(default)]
    force: bool,
    #[serde(default = "default_grace")]
    grace_period_ms: u64,
}

const fn default_grace() -> u64 {
    5_000
}

pub struct CloseApp {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl CloseApp {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "close_app",
                "Closes an application gracefully and verifies that it is no longer running",
                RiskLevel::R2,
                vec!["windows.apps.close"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["appId"],
                    "properties": {
                        "appId": {"type": "string", "minLength": 1, "maxLength": 64},
                        "force": {"type": "boolean", "default": false},
                        "gracePeriodMs": {"type": "integer", "minimum": 1000, "maximum": 30000, "default": 5000}
                    }
                }),
                json!({"type": "object", "properties": {"appId": {"type": "string"}, "closedProcesses": {"type": "integer"}, "forced": {"type": "boolean"}, "wasRunning": {"type": "boolean"}, "status": {"type": "string"}}}),
                Timeouts { default: Duration::from_secs(15), max: Duration::from_secs(45) },
                CancellationMode::Cooperative,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "process_absent",
                    timeout: Duration::from_secs(5),
                    poll_interval: Duration::from_millis(250),
                },
                Sensitivity::None,
                false,
            ),
        }
    }
}

#[async_trait]
impl Tool for CloseApp {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: CloseInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if !(1_000..=30_000).contains(&parsed.grace_period_ms) {
            return Err(ToolError::invalid("gracePeriodMs must be in 1000..=30000"));
        }
        let (id, entry) = resolve_app(&self.ctx, &parsed.app_id)?;
        if !entry.allow_close {
            return Err(ToolError::new(
                ErrorCode::NotPermitted,
                format!("the local catalog does not allow closing '{id}'"),
            ));
        }
        Ok(json!({ "appId": id, "force": parsed.force, "gracePeriodMs": parsed.grace_period_ms }))
    }

    /// `force` sube el riesgo a R3 (puede perder trabajo sin guardar).
    fn risk(&self, input: &Value) -> RiskLevel {
        if input.get("force").and_then(Value::as_bool).unwrap_or(false) { RiskLevel::R3 } else { RiskLevel::R2 }
    }

    fn required_permissions(&self, input: &Value) -> Vec<String> {
        let mut permissions = vec!["windows.apps.close".to_owned()];
        if input.get("force").and_then(Value::as_bool).unwrap_or(false) {
            permissions.push("windows.apps.close.force".to_owned());
        }
        permissions
    }

    fn resources(&self, input: &Value) -> Vec<ResourceLock> {
        input.get("appId").and_then(Value::as_str).map(|id| vec![ResourceLock::app(id)]).unwrap_or_default()
    }

    async fn execute(&self, input: &Value, ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (app_id, entry) = entry_of(&self.ctx, input)?;
        let force = input.get("force").and_then(Value::as_bool).unwrap_or(false);
        let grace = Duration::from_millis(input.get("gracePeriodMs").and_then(Value::as_u64).unwrap_or(5_000));
        let path = expected_path(&self.ctx, &entry);

        let processes = app_processes(&self.ctx, &entry, path.clone()).await?;
        if processes.is_empty() {
            return Ok(ExecutionOutcome {
                output: json!({
                    "appId": app_id, "closedProcesses": 0, "forced": false,
                    "wasRunning": false, "status": "NOT_RUNNING",
                }),
                details: Some(json!({ "method": "none" })),
                evidence: json!({ "wasRunning": false }),
            });
        }
        if let Some(protected) = processes.iter().find(|p| jarvis_windows_guard(&p.name)) {
            return Err(ToolError::new(
                ErrorCode::NotPermitted,
                format!("'{}' is a protected process", protected.name),
            ));
        }
        // Un proceso elevado no se puede cerrar desde integridad media: se informa sin tocar nada.
        if processes.iter().any(|p| p.elevated != Some(false)) {
            return Err(ToolError::new(
                ErrorCode::AppElevated,
                "the application runs elevated and cannot be closed by a standard-user agent",
            ));
        }

        let pids: Vec<u32> = processes.iter().map(|p| p.pid).collect();
        let signalled = {
            let pids = pids.clone();
            self.ctx.blocking(move |api| api.request_close(&pids)).await?
        };

        // Espera de cortesía: la app puede estar guardando o preguntando al usuario.
        let deadline = tokio::time::Instant::now() + grace;
        let mut remaining = processes.clone();
        while tokio::time::Instant::now() < deadline {
            ctx.sleep(Duration::from_millis(250)).await?;
            remaining = app_processes(&self.ctx, &entry, path.clone()).await?;
            if remaining.is_empty() {
                break;
            }
        }

        let mut closed = processes.len() - remaining.len();
        let mut forced = false;
        if !remaining.is_empty() && force {
            forced = true;
            for process in &remaining {
                let (pid, created) = (process.pid, process.created_at);
                match self.ctx.blocking(move |api| api.terminate(pid, created)).await {
                    Ok(()) => closed += 1,
                    Err(error) if error.code == ErrorCode::AccessDenied => {
                        return Err(ToolError::new(ErrorCode::AppElevated, "a process could not be terminated"));
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        let status = if remaining.is_empty() || forced { "CLOSED" } else { "CLOSE_REQUESTED" };
        Ok(ExecutionOutcome {
            output: json!({
                "appId": app_id,
                "closedProcesses": closed,
                "forced": forced,
                "wasRunning": true,
                "status": status,
            }),
            details: Some(
                json!({ "method": if forced { "terminate" } else { "wm_close" }, "windowsSignalled": signalled }),
            ),
            evidence: json!({ "wasRunning": true }),
        })
    }

    /// `process_absent`: ningún proceso de la app durante dos sondeos consecutivos.
    async fn verify(&self, input: &Value, _outcome: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
        let Ok((_, entry)) = entry_of(&self.ctx, input) else {
            return VerificationReport::inconclusive(Vec::new(), "the catalog entry disappeared");
        };
        let path = expected_path(&self.ctx, &entry);
        let deadline = tokio::time::Instant::now() + self.descriptor.verification.timeout;
        let mut consecutive_absences = 0;
        let mut remaining;
        loop {
            match app_processes(&self.ctx, &entry, path.clone()).await {
                Ok(processes) => {
                    remaining = processes.len();
                    if processes.is_empty() {
                        consecutive_absences += 1;
                        // Dos lecturas seguidas: evita dar por cerrada una app que se relanza sola.
                        if consecutive_absences >= 2 {
                            return VerificationReport::passed(vec![Check::new(
                                "no_process_for_app",
                                true,
                                Some(json!({ "remaining": 0 })),
                            )]);
                        }
                    } else {
                        consecutive_absences = 0;
                    }
                }
                Err(error) => return VerificationReport::inconclusive(Vec::new(), error.message),
            }
            if tokio::time::Instant::now() >= deadline {
                return VerificationReport::failed(
                    vec![Check::new("no_process_for_app", false, Some(json!({ "remaining": remaining })))],
                    "the application is still running; it may be waiting for user input",
                );
            }
            if ctx.sleep(self.descriptor.verification.poll_interval).await.is_err() {
                return VerificationReport::inconclusive(Vec::new(), "verification cancelled");
            }
        }
    }
}

/// Lista fija de procesos que nunca se cierran (no configurable desde el Cloud).
#[cfg(windows)]
fn jarvis_windows_guard(name: &str) -> bool {
    jarvis_windows::process::is_protected(name)
}

#[cfg(not(windows))]
fn jarvis_windows_guard(name: &str) -> bool {
    ["csrss.exe", "wininit.exe", "winlogon.exe", "lsass.exe", "services.exe", "smss.exe", "svchost.exe", "dwm.exe"]
        .iter()
        .any(|p| p.eq_ignore_ascii_case(name))
}
