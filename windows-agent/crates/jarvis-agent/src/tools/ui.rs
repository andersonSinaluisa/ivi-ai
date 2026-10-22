//! Tools de escritorio: traer una app al frente y avisar al usuario.
//!
//! `focus_window` toca el recurso `DESKTOP_INPUT` (foco, teclado y ratón son uno solo) y nunca
//! sintetiza pulsaciones para saltarse el *foreground lock* de Windows (ADR-002): si el sistema
//! deniega el foco, la verificación falla y se dice por qué.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jarvis_tooling::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationMode, VerificationSpec};
use jarvis_tooling::{
    Check, ErrorCode, ExecContext, ExecutionOutcome, ResourceLock, RiskLevel, Tool, ToolDescriptor, ToolError,
    VerificationReport,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ToolContext;
use super::apps::{app_processes, descriptor, entry_of, expected_path, resolve_app};

pub const MAX_TITLE_CHARS: usize = 64;
pub const MAX_BODY_CHARS: usize = 256;

// ---- focus_window -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FocusInput {
    app_id: String,
}

pub struct FocusWindow {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl FocusWindow {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "focus_window",
                "Brings an application from the local catalog to the foreground",
                RiskLevel::R2,
                vec!["windows.window.focus"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["appId"],
                    "properties": { "appId": {"type": "string", "minLength": 1, "maxLength": 64} }
                }),
                json!({"type": "object", "properties": {"appId": {"type": "string"}, "pid": {"type": "integer"}, "focused": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(5), max: Duration::from_secs(15) },
                CancellationMode::Cooperative,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "foreground_is",
                    timeout: Duration::from_secs(3),
                    poll_interval: Duration::from_millis(200),
                },
                Sensitivity::None,
                // Sin escritorio accesible no hay foco que dar.
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for FocusWindow {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: FocusInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        let (id, _) = resolve_app(&self.ctx, &parsed.app_id)?;
        Ok(json!({ "appId": id }))
    }

    fn resources(&self, _input: &Value) -> Vec<ResourceLock> {
        vec![ResourceLock::exclusive(ResourceLock::DESKTOP_INPUT)]
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (app_id, entry) = entry_of(&self.ctx, input)?;
        let session = self.ctx.blocking(|api| Ok(api.session_state())).await?;
        if !session.is_interactive() {
            return Err(ToolError::new(
                ErrorCode::SessionUnavailable,
                format!("the session is {} and no window can take the foreground", session.as_str()),
            ));
        }

        let path = expected_path(&self.ctx, &entry);
        let processes = app_processes(&self.ctx, &entry, path).await?;
        if processes.is_empty() {
            return Err(ToolError::new(ErrorCode::AppNotFound, "the application is not running"));
        }
        let pids: Vec<u32> = processes.iter().map(|process| process.pid).collect();

        let requested = pids.clone();
        let focused = self.ctx.blocking(move |api| api.focus_window(&requested)).await?;

        Ok(ExecutionOutcome {
            output: json!({ "appId": app_id, "pid": pids.first(), "focused": focused }),
            details: Some(json!({ "method": "set_foreground_window" })),
            evidence: json!({ "pids": pids }),
        })
    }

    /// `foreground_is`: la ventana en primer plano pertenece a la aplicación pedida.
    async fn verify(&self, _input: &Value, outcome: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
        let pids: Vec<u32> = outcome
            .evidence
            .get("pids")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_u64).map(|pid| pid as u32).collect())
            .unwrap_or_default();
        if pids.is_empty() {
            return VerificationReport::inconclusive(Vec::new(), "no process to check the foreground against");
        }

        let deadline = tokio::time::Instant::now() + self.descriptor.verification.timeout;
        loop {
            match self.ctx.blocking(move |api| api.foreground_pid()).await {
                Ok(Some(pid)) if pids.contains(&pid) => {
                    let checks = vec![Check::new("foreground_is", true, Some(json!({ "pid": pid })))];
                    return VerificationReport::passed(checks)
                        .with_output_patch(json!({ "pid": pid, "focused": true }));
                }
                Ok(_) => {}
                Err(error) => return VerificationReport::inconclusive(Vec::new(), error.message),
            }
            if tokio::time::Instant::now() >= deadline {
                let checks = vec![Check::new("foreground_is", false, None)];
                return VerificationReport::failed(
                    checks,
                    "Windows kept the foreground on another application (foreground lock): \
                     the agent does not force it by synthesising input",
                )
                .with_output_patch(json!({ "focused": false }));
            }
            if ctx.sleep(self.descriptor.verification.poll_interval).await.is_err() {
                return VerificationReport::inconclusive(Vec::new(), "verification cancelled");
            }
        }
    }
}

// ---- show_notification ------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NotifyInput {
    title: String,
    body: String,
}

/// Texto plano: sin caracteres de control ni marcado, y recortado.
fn plain_text(raw: &str, limit: usize) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= limit {
        cleaned.to_owned()
    } else {
        cleaned.chars().take(limit - 1).collect::<String>() + "…"
    }
}

pub struct ShowNotification {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl ShowNotification {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "show_notification",
                "Shows a plain-text notification to the user, with no action buttons",
                RiskLevel::R1,
                vec!["windows.notify"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "body"],
                    "properties": {
                        "title": {"type": "string", "minLength": 1, "maxLength": 64},
                        "body": {"type": "string", "minLength": 1, "maxLength": 256}
                    }
                }),
                json!({"type": "object", "properties": {"delivered": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(3), max: Duration::from_secs(10) },
                CancellationMode::None,
                // La entrega la confirma la propia API al mostrarlo (ADR-017): no hay un segundo
                // observable que mirar después, así que no se declara una verificación aparte.
                VerificationSpec::not_applicable(),
                // El texto lo eligió quien manda la tarea; el usuario lo ve, no es dato suyo.
                Sensitivity::None,
                true,
            ),
        }
    }
}

#[async_trait]
impl Tool for ShowNotification {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: NotifyInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        let title = plain_text(&parsed.title, MAX_TITLE_CHARS);
        let body = plain_text(&parsed.body, MAX_BODY_CHARS);
        if title.is_empty() || body.is_empty() {
            return Err(ToolError::invalid("title and body must contain visible text"));
        }
        Ok(json!({ "title": title, "body": body }))
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let title = input.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
        let body = input.get("body").and_then(Value::as_str).unwrap_or_default().to_owned();

        let session = self.ctx.blocking(|api| Ok(api.session_state())).await?;
        if !session.is_interactive() {
            return Err(ToolError::new(
                ErrorCode::SessionUnavailable,
                format!("the session is {} and the user would not see the notification", session.as_str()),
            ));
        }

        self.ctx.blocking(move |api| api.show_notification(&title, &body)).await.map_err(|error| {
            ToolError::new(ErrorCode::ExecutionFailed, format!("the notification was not shown: {}", error.message))
        })?;

        Ok(ExecutionOutcome {
            output: json!({ "delivered": true }),
            details: Some(json!({ "method": "notification_area_balloon" })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_text_is_plain_and_bounded() {
        assert_eq!(plain_text("  hola\r\nmundo  ", 64), "holamundo");
        assert_eq!(plain_text("<toast><audio src='x'/></toast>", 64), "<toast><audio src='x'/></toast>");
        let long = "x".repeat(300);
        assert_eq!(plain_text(&long, MAX_BODY_CHARS).chars().count(), MAX_BODY_CHARS);
    }
}
