use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use crate::domain::{RiskClass, ToolOutput, ToolSource, ToolSpec};
use crate::tools::{validate_object, Tool, ToolContext, ToolError};

const SCHEMA: &str = include_str!("../../../../../../packages/tool-schemas/schemas/get_current_time.json");

pub struct GetCurrentTime {
    spec: ToolSpec,
}

impl GetCurrentTime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                id: "local.get_current_time".into(),
                name: "get_current_time".into(),
                description: "Devuelve la fecha y hora actuales de la computadora, con zona horaria.".into(),
                parameters: super::schema(SCHEMA),
                risk: RiskClass::ReadOnly,
                permissions: vec!["system.clock.read".into()],
                timeout_ms: 1_000,
                cancellable: false,
                source: ToolSource::Local,
                untrusted_output: false,
            },
        }
    }
}

impl Default for GetCurrentTime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetCurrentTime {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate_object(&self.spec.parameters, arguments).map_err(ToolError::InvalidArguments)?;
        Ok(json!({}))
    }

    async fn execute(&self, _arguments: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let now = Local::now();
        Ok(ToolOutput::success(json!({
            "iso": now.to_rfc3339(),
            "date": now.format("%Y-%m-%d").to_string(),
            "time": now.format("%H:%M:%S").to_string(),
            "weekday": now.format("%A").to_string(),
            "utcOffset": now.format("%:z").to_string(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn returns_a_parseable_timestamp() {
        let t = GetCurrentTime::new();
        let ctx = ToolContext { run_id: "r".into(), call_id: "c".into(), cancel: CancellationToken::new() };
        let out = t.execute(json!({}), &ctx).await.expect("ok");
        assert!(out.ok);
        let iso = out.data["iso"].as_str().expect("iso");
        assert!(chrono::DateTime::parse_from_rfc3339(iso).is_ok());
        assert!(t.validate(&json!({"x": 1})).is_err(), "no acepta parámetros");
    }
}
