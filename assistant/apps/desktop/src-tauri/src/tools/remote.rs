//! Herramientas del windows-agent expuestas como `Tool` locales (proxy por la Local Tool API).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{validate_object, Tool, ToolContext, ToolError};
use crate::domain::{RiskClass, ToolOutput, ToolSource, ToolSpec, VerificationStatus};
use crate::infrastructure::windows::{RemoteToolDescriptor, ToolApiClient, ToolApiError};

/// Herramientas cuyo resultado puede traer texto de terceros (posible prompt injection).
const UNTRUSTED_HINTS: &[&str] =
    &["read", "file", "directory", "url", "browser", "web", "clipboard", "window", "text", "element", "command", "ui"];

pub struct RemoteTool {
    spec: ToolSpec,
    client: Arc<ToolApiClient>,
}

impl RemoteTool {
    #[must_use]
    pub fn new(descriptor: RemoteToolDescriptor, client: Arc<ToolApiClient>) -> Self {
        let sensitive_output = descriptor
            .sensitivity
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|s| s != "none");
        let untrusted = sensitive_output || UNTRUSTED_HINTS.iter().any(|h| descriptor.name.contains(h));
        let parameters = if descriptor.input_schema.get("type").and_then(Value::as_str) == Some("object") {
            descriptor.input_schema
        } else {
            json!({"type": "object", "properties": {}})
        };
        Self {
            spec: ToolSpec {
                id: format!("windows-agent.{}", descriptor.name),
                description: if descriptor.description.is_empty() {
                    format!("Herramienta '{}' del windows-agent", descriptor.name)
                } else {
                    descriptor.description
                },
                name: descriptor.name,
                parameters,
                risk: RiskClass::from_agent_level(&descriptor.risk_level),
                permissions: descriptor.required_permissions,
                // El windows-agent aplica su propio timeout; este es el máximo que esperamos nosotros.
                timeout_ms: 60_000,
                cancellable: false,
                source: ToolSource::WindowsAgent,
                untrusted_output: untrusted,
            },
            client,
        }
    }
}

/// Traduce el `ToolResult` (jarvis.tool-result/1) del windows-agent.
#[must_use]
pub fn map_tool_result(result: &Value) -> ToolOutput {
    let ok = result.get("outcome").and_then(Value::as_str) == Some("SUCCEEDED");
    let verification = match result.pointer("/verification/status").and_then(Value::as_str) {
        Some("PASSED") => Some(VerificationStatus::Passed),
        Some("FAILED") => Some(VerificationStatus::Failed),
        Some("INCONCLUSIVE") => Some(VerificationStatus::Inconclusive),
        Some("NOT_APPLICABLE" | "SKIPPED") => Some(VerificationStatus::NotApplicable),
        _ => None,
    };
    let error = result.pointer("/error/message").and_then(Value::as_str).map(str::to_owned);
    ToolOutput {
        ok,
        data: json!({
            "output": result.get("output").cloned().unwrap_or(Value::Null),
            "checks": result.pointer("/verification/checks").cloned().unwrap_or(Value::Null),
        }),
        error: if ok { None } else { error.or_else(|| Some("la herramienta no tuvo éxito".into())) },
        verification,
    }
}

#[async_trait]
impl Tool for RemoteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate_object(&self.spec.parameters, arguments).map_err(ToolError::InvalidArguments)?;
        Ok(if arguments.is_null() { json!({}) } else { arguments.clone() })
    }

    async fn execute(&self, arguments: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let invocation_id = format!("{}:{}", ctx.run_id, ctx.call_id);
        match self.client.call(&self.spec.name, arguments, &invocation_id).await {
            Ok(result) => Ok(map_tool_result(&result)),
            Err(ToolApiError::ToolFailed { message, data }) => {
                Ok(data.as_ref().map_or_else(|| ToolOutput::failure(message), map_tool_result))
            }
            Err(ToolApiError::NotPermitted(m)) => Err(ToolError::NotPermitted(m)),
            Err(ToolApiError::Unavailable(m)) => Err(ToolError::Unavailable(m)),
            Err(e) => Err(ToolError::Failed(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::windows::tool_api::testing::fake_agent;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn proxies_calls_and_maps_risk_and_results() {
        let client = Arc::new(ToolApiClient::new(fake_agent(None), Duration::from_secs(1)));
        let tools: Vec<RemoteTool> =
            client.list_tools().await.expect("list").into_iter().map(|d| RemoteTool::new(d, client.clone())).collect();
        assert_eq!(tools[0].spec().risk, RiskClass::ReadOnly);
        assert_eq!(tools[1].spec().risk, RiskClass::Sensitive);

        let ctx = ToolContext { run_id: "r".into(), call_id: "c".into(), cancel: CancellationToken::new() };
        let out = tools[0].execute(json!({}), &ctx).await.expect("ok");
        assert!(out.ok);
        assert_eq!(out.verification, Some(VerificationStatus::Passed));
        assert_eq!(out.data["output"]["processes"][0], "notepad.exe");

        let err = tools[1].execute(json!({"appId": "notepad"}), &ctx).await.expect_err("denied");
        assert!(matches!(err, ToolError::NotPermitted(_)));
    }

    #[test]
    fn third_party_output_is_marked_untrusted() {
        let client = Arc::new(ToolApiClient::new(fake_agent(None), Duration::from_secs(1)));
        let mk = |name: &str| {
            RemoteTool::new(
                RemoteToolDescriptor {
                    name: name.into(),
                    version: None,
                    description: "x".into(),
                    input_schema: json!({"type": "object"}),
                    risk_level: "R0".into(),
                    required_permissions: vec![],
                    sensitivity: None,
                },
                client.clone(),
            )
        };
        for name in ["read_file", "list_directory", "get_text", "run_command", "find_element"] {
            assert!(mk(name).spec().untrusted_output, "{name}");
        }
        assert!(!mk("is_app_running").spec().untrusted_output);
    }

    #[test]
    fn failed_results_carry_the_error() {
        let out = map_tool_result(&json!({
            "outcome": "FAILED",
            "error": {"code": "APP_NOT_FOUND", "message": "app no está en el catálogo"},
            "verification": {"status": "SKIPPED", "checks": []}
        }));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("app no está en el catálogo"));
        assert_eq!(out.verification, Some(VerificationStatus::NotApplicable));
    }
}
