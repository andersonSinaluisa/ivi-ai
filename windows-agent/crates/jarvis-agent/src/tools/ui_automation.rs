//! Tools de UI Automation (HU-WIN-065, HU-WIN-066; docs/05-windows/ui-automation.md).
//!
//! Se actúa por **semántica**, nunca por coordenadas ni teclado sintético: se localiza el elemento
//! por `automationId`/`controlType`/`name` y se usa el patrón UIA que expone. Un selector que
//! encaja con más de un elemento es un error (`ELEMENT_AMBIGUOUS`), no se elige el primero.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jarvis_tooling::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationMode, VerificationSpec};
use jarvis_tooling::platform::{UiElement, UiSelector};
use jarvis_tooling::{
    Check, ErrorCode, ExecContext, ExecutionOutcome, ResourceLock, RiskLevel, Tool, ToolDescriptor, ToolError,
    VerificationReport,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ToolContext;
use super::apps::{app_processes, descriptor, entry_of, expected_path, resolve_app};
use crate::logging::Sensitive;

/// Plazo de cada llamada al host UIA. Una app colgada no puede bloquear al agente.
const UIA_TIMEOUT: Duration = Duration::from_secs(5);
/// Cuántas coincidencias se piden para poder detectar ambigüedad sin traer la UI entera.
const AMBIGUITY_PROBE: usize = 5;
/// Palabras que convierten una acción en R3: el efecto no es trivialmente reversible.
const DESTRUCTIVE_WORDS: &[&str] =
    &["delete", "eliminar", "borrar", "send", "enviar", "pay", "pagar", "transfer", "transferir", "format", "shutdown"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SelectorInput {
    #[serde(default)]
    automation_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    class_name: Option<String>,
    #[serde(default)]
    control_type: Option<String>,
}

fn selector_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "automationId": {"type": "string", "maxLength": 128},
            "name": {"type": "string", "maxLength": 256},
            "className": {"type": "string", "maxLength": 128},
            "controlType": {"type": "string", "maxLength": 32}
        }
    })
}

/// Valida el selector: algo tiene que restringir, y el tipo de control tiene que existir.
fn validate_selector(input: &Value) -> Result<Value, ToolError> {
    let parsed: SelectorInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
    let identifying = parsed.automation_id.is_some() || parsed.name.is_some() || parsed.class_name.is_some();
    if !identifying {
        return Err(ToolError::invalid("the selector needs automationId, name or className"));
    }
    for (field, value) in
        [("automationId", &parsed.automation_id), ("name", &parsed.name), ("className", &parsed.class_name)]
    {
        if value.as_ref().is_some_and(|v| v.trim().is_empty()) {
            return Err(ToolError::invalid(format!("{field} cannot be empty")));
        }
    }
    if let Some(control) = &parsed.control_type
        && control_type_is_unknown(control)
    {
        return Err(ToolError::invalid(format!("unknown controlType '{control}'")));
    }
    Ok(json!({
        "automationId": parsed.automation_id,
        "name": parsed.name,
        "className": parsed.class_name,
        "controlType": parsed.control_type,
    }))
}

#[cfg(windows)]
fn control_type_is_unknown(name: &str) -> bool {
    jarvis_windows::uia::control_type_id(name).is_none()
}

#[cfg(not(windows))]
const fn control_type_is_unknown(_name: &str) -> bool {
    false
}

fn selector_of(input: &Value) -> UiSelector {
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
    UiSelector {
        automation_id: text("automationId"),
        name: text("name"),
        class_name: text("className"),
        control_type: text("controlType"),
    }
}

/// Ventana de la aplicación del catálogo sobre la que se va a trabajar.
async fn window_of(ctx: &ToolContext, input: &Value) -> Result<(isize, u32), ToolError> {
    let (_, entry) = entry_of(ctx, input)?;
    let session = ctx.blocking(|api| Ok(api.session_state())).await?;
    if !session.is_interactive() {
        return Err(ToolError::new(
            ErrorCode::SessionUnavailable,
            format!("the session is {} and its UI cannot be automated", session.as_str()),
        ));
    }
    let path = expected_path(ctx, &entry);
    let processes = app_processes(ctx, &entry, path).await?;
    if processes.is_empty() {
        return Err(ToolError::new(ErrorCode::AppNotFound, "the application is not running"));
    }
    if processes.iter().any(|process| process.elevated == Some(true)) {
        // UIPI: desde integridad media no se automatiza una app elevada, y fingirlo sería peor.
        return Err(ToolError::new(
            ErrorCode::AppElevated,
            "the application runs elevated and cannot be automated from a standard-user agent",
        ));
    }
    let pids: Vec<u32> = processes.iter().map(|process| process.pid).collect();
    let windows = ctx.blocking(move |api| api.top_level_windows()).await?;
    let window = windows
        .into_iter()
        .find(|window| pids.contains(&window.pid))
        .ok_or_else(|| ToolError::new(ErrorCode::ElementNotFound, "the application has no visible window"))?;
    Ok((window.handle, window.pid))
}

fn ui_error(error: ToolError) -> ToolError {
    // El host UIA distingue "no hay" de "hay varios" por el mensaje; aquí se traduce al catálogo.
    if error.message.contains("no element matches") {
        ToolError::new(ErrorCode::ElementNotFound, "no element matches the selector")
    } else if error.message.contains("matches") && error.message.contains("elements") {
        ToolError::new(ErrorCode::ElementAmbiguous, error.message)
    } else {
        error
    }
}

/// Busca el elemento único que cumple el selector.
async fn single_element(ctx: &ToolContext, hwnd: isize, selector: UiSelector) -> Result<UiElement, ToolError> {
    let found =
        ctx.blocking(move |api| api.ui_find(hwnd, &selector, AMBIGUITY_PROBE, UIA_TIMEOUT)).await.map_err(ui_error)?;
    match found.len() {
        0 => Err(ToolError::new(ErrorCode::ElementNotFound, "no element matches the selector")),
        1 => Ok(found.into_iter().next().unwrap_or_else(|| unreachable!("len == 1"))),
        more => Err(ToolError::new(
            ErrorCode::ElementAmbiguous,
            format!("the selector matches {more} elements: make it more specific"),
        )),
    }
}

/// Descripción publicable de un elemento (el `name` puede ser personal: se redacta aparte).
fn element_json(element: &UiElement, name: Option<&str>) -> Value {
    json!({
        "automationId": element.automation_id,
        "name": name,
        "className": element.class_name,
        "controlType": element.control_type,
        "enabled": element.enabled,
        "patterns": element.patterns,
        "bounds": {"x": element.bounds.0, "y": element.bounds.1, "width": element.bounds.2, "height": element.bounds.3},
        "value": element.value,
        "toggled": element.toggled,
        "selected": element.selected,
    })
}

/// Aplica la política de títulos al nombre visible de un elemento o ventana.
fn redacted(ctx: &ToolContext, process: Option<&str>, text: &str) -> (Option<String>, bool) {
    let policy = ctx.policy();
    super::privacy::apply(&policy.privacy, process, Some(text))
}

// ---- find_window ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FindWindowInput {
    app_id: String,
}

pub struct FindWindow {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl FindWindow {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "find_window",
                "Finds the main window of an application from the local catalog",
                RiskLevel::R1,
                vec!["windows.ui.read"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["appId"],
                    "properties": { "appId": {"type": "string", "minLength": 1, "maxLength": 64} }
                }),
                json!({"type": "object", "properties": {"appId": {"type": "string"}, "found": {"type": "boolean"}, "pid": {"type": "integer"}, "title": {"type": "string"}, "className": {"type": "string"}}}),
                Timeouts { default: Duration::from_secs(5), max: Duration::from_secs(15) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                // El título es dato del usuario.
                Sensitivity::Output,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for FindWindow {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: FindWindowInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        let (id, _) = resolve_app(&self.ctx, &parsed.app_id)?;
        Ok(json!({ "appId": id }))
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let app_id = input.get("appId").and_then(Value::as_str).unwrap_or_default().to_owned();
        let (hwnd, pid) = window_of(&self.ctx, input).await?;
        let window = self.ctx.blocking(move |api| api.ui_window(hwnd, UIA_TIMEOUT)).await?;
        let (title, was_redacted) = redacted(&self.ctx, Some(&app_id), &window.name);
        let title = title.map(Sensitive::new);

        Ok(ExecutionOutcome {
            output: json!({
                "appId": app_id,
                "found": true,
                "pid": pid,
                "title": title.as_ref().map(Sensitive::expose),
                "titleRedacted": was_redacted,
                "className": window.class_name,
            }),
            details: Some(json!({ "method": "uia_element_from_handle" })),
            ..Default::default()
        })
    }
}

// ---- find_element -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ElementInput {
    app_id: String,
    selector: Value,
}

fn element_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["appId", "selector"],
        "properties": {
            "appId": {"type": "string", "minLength": 1, "maxLength": 64},
            "selector": selector_schema()
        }
    })
}

fn validate_element_input(ctx: &ToolContext, input: &Value) -> Result<Value, ToolError> {
    let parsed: ElementInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
    let (id, _) = resolve_app(ctx, &parsed.app_id)?;
    Ok(json!({ "appId": id, "selector": validate_selector(&parsed.selector)? }))
}

pub struct FindElement {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl FindElement {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "find_element",
                "Finds a single UI element by its semantic selector",
                RiskLevel::R1,
                vec!["windows.ui.read"],
                element_input_schema(),
                json!({"type": "object", "properties": {"element": {"type": "object"}}}),
                Timeouts { default: Duration::from_secs(5), max: Duration::from_secs(20) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                Sensitivity::Output,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for FindElement {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        validate_element_input(&self.ctx, input)
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (hwnd, _) = window_of(&self.ctx, input).await?;
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));
        let element = single_element(&self.ctx, hwnd, selector).await?;
        let (name, was_redacted) = redacted(&self.ctx, None, &element.name);

        Ok(ExecutionOutcome {
            output: json!({
                "element": element_json(&element, name.as_deref()),
                "nameRedacted": was_redacted,
            }),
            details: Some(json!({ "method": "uia_find_all" })),
            ..Default::default()
        })
    }
}

// ---- get_text ---------------------------------------------------------------------

pub struct GetText {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl GetText {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "get_text",
                "Reads the text of a UI element (ValuePattern or its name)",
                RiskLevel::R1,
                vec!["windows.ui.read"],
                element_input_schema(),
                json!({"type": "object", "properties": {"text": {"type": "string"}, "source": {"type": "string"}}}),
                Timeouts { default: Duration::from_secs(5), max: Duration::from_secs(20) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                // Puede leer cualquier texto visible: nunca a los logs, redactado tras settled.
                Sensitivity::InputAndOutput,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for GetText {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        validate_element_input(&self.ctx, input)
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (hwnd, _) = window_of(&self.ctx, input).await?;
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));
        let element = single_element(&self.ctx, hwnd, selector).await?;

        // `ValuePattern` primero; si no lo expone, el nombre accesible es lo que ve el usuario.
        let (text, source) = match &element.value {
            Some(value) => (value.clone(), "ValuePattern"),
            None => (element.name.clone(), "Name"),
        };
        let text = Sensitive::new(text);

        Ok(ExecutionOutcome {
            output: json!({ "text": text.expose(), "source": source, "controlType": element.control_type }),
            details: Some(json!({ "method": "uia_read" })),
            ..Default::default()
        })
    }
}

// ---- invoke_element ---------------------------------------------------------------

pub struct InvokeElement {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

/// `true` si el nombre del elemento sugiere una acción difícil de deshacer.
fn is_destructive(selector: &Value) -> bool {
    let text = [selector.get("name"), selector.get("automationId")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    DESTRUCTIVE_WORDS.iter().any(|word| text.contains(word))
}

impl InvokeElement {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "invoke_element",
                "Activates a UI element using its UI Automation pattern (never a synthetic click)",
                RiskLevel::R2,
                vec!["windows.ui.act"],
                element_input_schema(),
                json!({"type": "object", "properties": {"pattern": {"type": "string"}, "changed": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(10), max: Duration::from_secs(30) },
                CancellationMode::Cooperative,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "observable_change",
                    timeout: Duration::from_secs(4),
                    poll_interval: Duration::from_millis(250),
                },
                Sensitivity::Output,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for InvokeElement {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        validate_element_input(&self.ctx, input)
    }

    /// Una acción con pinta de destructiva sube a R3 (pide aprobación del owner).
    fn risk(&self, input: &Value) -> RiskLevel {
        if input.get("selector").is_some_and(is_destructive) { RiskLevel::R3 } else { RiskLevel::R2 }
    }

    fn resources(&self, _input: &Value) -> Vec<ResourceLock> {
        vec![ResourceLock::exclusive(ResourceLock::DESKTOP_INPUT)]
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (hwnd, _) = window_of(&self.ctx, input).await?;
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));

        // Estado previo: sin él no habría forma de comprobar que la acción hizo algo.
        let before = single_element(&self.ctx, hwnd, selector.clone()).await?;
        let windows_before = self.ctx.blocking(move |api| api.top_level_windows()).await?.len();

        let acting = selector.clone();
        let pattern =
            self.ctx.blocking(move |api| api.ui_invoke(hwnd, &acting, UIA_TIMEOUT)).await.map_err(ui_error)?;

        Ok(ExecutionOutcome {
            output: json!({ "pattern": pattern }),
            details: Some(json!({ "method": "uia_pattern" })),
            evidence: json!({
                "hwnd": hwnd,
                "value": before.value,
                "toggled": before.toggled,
                "selected": before.selected,
                "windows": windows_before,
            }),
        })
    }

    /// `observable_change`: algo del elemento o del escritorio tiene que haber cambiado.
    async fn verify(&self, input: &Value, outcome: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
        let Some(hwnd) = outcome.evidence.get("hwnd").and_then(Value::as_i64) else {
            return VerificationReport::inconclusive(Vec::new(), "no window to observe");
        };
        let hwnd = hwnd as isize;
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));
        let deadline = tokio::time::Instant::now() + self.descriptor.verification.timeout;

        loop {
            let probe = selector.clone();
            let found = self.ctx.blocking(move |api| api.ui_find(hwnd, &probe, 1, UIA_TIMEOUT)).await;
            let windows = self.ctx.blocking(move |api| api.top_level_windows()).await.map(|w| w.len());

            let change = match (&found, &windows) {
                // El elemento desapareció: es un cambio observable (diálogo cerrado, ítem borrado).
                (Ok(elements), _) if elements.is_empty() => Some("element_gone"),
                (Ok(elements), _) => {
                    let now = &elements[0];
                    let changed = outcome.evidence.get("value").map(Value::as_str) != Some(now.value.as_deref())
                        || outcome.evidence.get("toggled").and_then(Value::as_bool) != now.toggled
                        || outcome.evidence.get("selected").and_then(Value::as_bool) != now.selected;
                    if changed {
                        Some("element_state")
                    } else if windows.as_ref().is_ok_and(|count| {
                        outcome.evidence.get("windows").and_then(Value::as_u64).map(|b| b as usize) != Some(*count)
                    }) {
                        Some("window_opened_or_closed")
                    } else {
                        None
                    }
                }
                (Err(error), _) => return VerificationReport::inconclusive(Vec::new(), error.message.clone()),
            };

            if let Some(what) = change {
                let checks = vec![Check::new("observable_change", true, Some(json!({ "change": what })))];
                return VerificationReport::passed(checks).with_output_patch(json!({ "changed": true }));
            }
            if tokio::time::Instant::now() >= deadline {
                let checks = vec![Check::new("observable_change", false, None)];
                return VerificationReport::failed(
                    checks,
                    "the element accepted the action but nothing observable changed",
                )
                .with_output_patch(json!({ "changed": false }));
            }
            if ctx.sleep(self.descriptor.verification.poll_interval).await.is_err() {
                return VerificationReport::inconclusive(Vec::new(), "verification cancelled");
            }
        }
    }
}

// ---- set_text ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetTextInput {
    app_id: String,
    selector: Value,
    text: String,
}

pub struct SetText {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl SetText {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "set_text",
                "Writes text into a UI element using ValuePattern (never synthetic keystrokes)",
                RiskLevel::R2,
                vec!["windows.ui.act"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["appId", "selector", "text"],
                    "properties": {
                        "appId": {"type": "string", "minLength": 1, "maxLength": 64},
                        "selector": selector_schema(),
                        "text": {"type": "string", "maxLength": 4096}
                    }
                }),
                json!({"type": "object", "properties": {"written": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(10), max: Duration::from_secs(30) },
                CancellationMode::Cooperative,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "value_equals",
                    timeout: Duration::from_secs(4),
                    poll_interval: Duration::from_millis(250),
                },
                // El texto escrito es dato del usuario.
                Sensitivity::InputAndOutput,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for SetText {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: SetTextInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if parsed.text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err(ToolError::invalid("the text contains control characters"));
        }
        let (id, _) = resolve_app(&self.ctx, &parsed.app_id)?;
        Ok(json!({ "appId": id, "selector": validate_selector(&parsed.selector)?, "text": parsed.text }))
    }

    fn risk(&self, input: &Value) -> RiskLevel {
        if input.get("selector").is_some_and(is_destructive) { RiskLevel::R3 } else { RiskLevel::R2 }
    }

    fn resources(&self, _input: &Value) -> Vec<ResourceLock> {
        vec![ResourceLock::exclusive(ResourceLock::DESKTOP_INPUT)]
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (hwnd, _) = window_of(&self.ctx, input).await?;
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));
        let text = input.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();

        // Que exista y sea único se comprueba antes de escribir nada.
        let _ = single_element(&self.ctx, hwnd, selector.clone()).await?;

        let writing = selector.clone();
        let written = text.clone();
        self.ctx
            .blocking(move |api| api.ui_set_value(hwnd, &writing, &written, UIA_TIMEOUT))
            .await
            .map_err(ui_error)?;

        Ok(ExecutionOutcome {
            output: json!({ "written": true }),
            details: Some(json!({ "method": "uia_value_pattern", "length": text.chars().count() })),
            evidence: json!({ "hwnd": hwnd, "text": text }),
        })
    }

    /// `value_equals`: el elemento tiene exactamente el texto que se pidió escribir.
    async fn verify(&self, input: &Value, outcome: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
        let Some(hwnd) = outcome.evidence.get("hwnd").and_then(Value::as_i64) else {
            return VerificationReport::inconclusive(Vec::new(), "no window to observe");
        };
        let hwnd = hwnd as isize;
        let expected = outcome.evidence.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
        let selector = selector_of(input.get("selector").unwrap_or(&Value::Null));
        let deadline = tokio::time::Instant::now() + self.descriptor.verification.timeout;

        loop {
            let probe = selector.clone();
            match self.ctx.blocking(move |api| api.ui_find(hwnd, &probe, 1, UIA_TIMEOUT)).await {
                Ok(elements) if elements.first().is_some_and(|e| e.value.as_deref() == Some(expected.as_str())) => {
                    // El propio texto no se publica en el check: solo su longitud.
                    let checks =
                        vec![Check::new("value_equals", true, Some(json!({ "length": expected.chars().count() })))];
                    return VerificationReport::passed(checks);
                }
                Ok(_) => {}
                Err(error) => return VerificationReport::inconclusive(Vec::new(), error.message),
            }
            if tokio::time::Instant::now() >= deadline {
                let checks = vec![Check::new("value_equals", false, None)];
                return VerificationReport::failed(checks, "the element does not hold the requested text");
            }
            if ctx.sleep(self.descriptor.verification.poll_interval).await.is_err() {
                return VerificationReport::inconclusive(Vec::new(), "verification cancelled");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selector_must_identify_something() {
        assert!(validate_selector(&json!({})).is_err(), "sin nada que restrinja");
        assert!(validate_selector(&json!({"controlType": "Button"})).is_err(), "solo el tipo no identifica");
        assert!(validate_selector(&json!({"automationId": "  "})).is_err(), "vacío no vale");
        assert!(validate_selector(&json!({"automationId": "SaveButton"})).is_ok());
        assert!(validate_selector(&json!({"name": "Guardar", "controlType": "Button"})).is_ok());
    }

    #[test]
    fn coordinates_are_not_part_of_the_contract() {
        // Un selector con coordenadas se rechaza por esquema estricto, no se ignora en silencio.
        assert!(validate_selector(&json!({"x": 100, "y": 200})).is_err());
        assert!(validate_selector(&json!({"automationId": "Save", "x": 10})).is_err());
    }

    #[test]
    fn destructive_looking_actions_are_treated_as_r3() {
        assert!(is_destructive(&json!({"name": "Delete account"})));
        assert!(is_destructive(&json!({"name": "Enviar"})));
        assert!(is_destructive(&json!({"automationId": "payNowButton"})));
        assert!(!is_destructive(&json!({"name": "Save"})));
        assert!(!is_destructive(&json!({"automationId": "num2Button"})));
    }
}
