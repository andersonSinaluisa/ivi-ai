use async_trait::async_trait;
use serde_json::{json, Value};
use sysinfo::System;

use crate::domain::{RiskClass, ToolOutput, ToolSource, ToolSpec};
use crate::tools::{validate_object, Tool, ToolContext, ToolError};

const SCHEMA: &str = include_str!("../../../../../../packages/tool-schemas/schemas/get_system_info.json");

pub struct GetSystemInfo {
    spec: ToolSpec,
}

impl GetSystemInfo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                id: "local.get_system_info".into(),
                name: "get_system_info".into(),
                description: "Devuelve información básica del equipo: sistema operativo, nombre del equipo, CPU, memoria y tiempo encendido.".into(),
                parameters: super::schema(SCHEMA),
                risk: RiskClass::ReadOnly,
                permissions: vec!["system.info.read".into()],
                timeout_ms: 5_000,
                cancellable: false,
                source: ToolSource::Local,
                untrusted_output: false,
            },
        }
    }
}

impl Default for GetSystemInfo {
    fn default() -> Self {
        Self::new()
    }
}

const MIB: u64 = 1024 * 1024;

#[async_trait]
impl Tool for GetSystemInfo {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate_object(&self.spec.parameters, arguments).map_err(ToolError::InvalidArguments)?;
        Ok(json!({}))
    }

    async fn execute(&self, _arguments: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let data = tokio::task::spawn_blocking(|| {
            let mut sys = System::new();
            sys.refresh_memory();
            sys.refresh_cpu_all();
            let cpus = sys.cpus();
            json!({
                "os": System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned()),
                "kernel": System::kernel_version(),
                "hostname": System::host_name(),
                "arch": std::env::consts::ARCH,
                "cpu": cpus.first().map(|c| c.brand().trim().to_owned()),
                "logicalCpus": cpus.len(),
                "memoryTotalMiB": sys.total_memory() / MIB,
                "memoryUsedMiB": sys.used_memory() / MIB,
                "uptimeSeconds": System::uptime(),
            })
        })
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(ToolOutput::success(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn reports_basic_fields() {
        let ctx = ToolContext { run_id: "r".into(), call_id: "c".into(), cancel: CancellationToken::new() };
        let out = GetSystemInfo::new().execute(json!({}), &ctx).await.expect("ok");
        assert!(out.data["logicalCpus"].as_u64().unwrap_or(0) >= 1);
        assert!(out.data["memoryTotalMiB"].as_u64().unwrap_or(0) > 0);
        assert!(out.data["os"].is_string());
    }
}
