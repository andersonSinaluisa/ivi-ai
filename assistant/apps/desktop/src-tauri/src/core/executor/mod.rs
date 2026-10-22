//! EXECUTE: corre una herramienta ya autorizada con su tiempo máximo y cancelación.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::ToolOutput;
use crate::tools::{Tool, ToolContext, ToolError};

#[derive(Debug, Clone)]
pub struct Execution {
    pub output: Result<ToolOutput, ToolError>,
    pub duration: Duration,
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn execute(&self, tool: Arc<dyn Tool>, arguments: Value, ctx: ToolContext) -> Execution;
}

#[derive(Default)]
pub struct DefaultExecutor;

#[async_trait]
impl Executor for DefaultExecutor {
    async fn execute(&self, tool: Arc<dyn Tool>, arguments: Value, ctx: ToolContext) -> Execution {
        let started = Instant::now();
        let timeout_ms = tool.spec().timeout_ms;
        let cancel = ctx.cancel.clone();
        let output = tokio::select! {
            () = cancel.cancelled() => Err(ToolError::Cancelled),
            r = tokio::time::timeout(Duration::from_millis(timeout_ms), tool.execute(arguments, &ctx)) => {
                r.unwrap_or(Err(ToolError::Timeout(timeout_ms)))
            }
        };
        Execution { output, duration: started.elapsed() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RiskClass, ToolSource, ToolSpec};
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    struct Slow(ToolSpec);

    #[async_trait]
    impl Tool for Slow {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn validate(&self, a: &Value) -> Result<Value, ToolError> {
            Ok(a.clone())
        }
        async fn execute(&self, _a: Value, _c: &ToolContext) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(ToolOutput::success(json!(1)))
        }
    }

    fn slow(timeout_ms: u64) -> Arc<dyn Tool> {
        Arc::new(Slow(ToolSpec {
            id: "slow".into(),
            name: "slow".into(),
            description: "lenta".into(),
            parameters: json!({"type": "object"}),
            risk: RiskClass::ReadOnly,
            permissions: vec![],
            timeout_ms,
            cancellable: true,
            source: ToolSource::Local,
            untrusted_output: false,
        }))
    }

    #[tokio::test]
    async fn enforces_the_timeout() {
        let ctx = ToolContext { run_id: "r".into(), call_id: "c".into(), cancel: CancellationToken::new() };
        let e = DefaultExecutor.execute(slow(50), json!({}), ctx).await;
        assert_eq!(e.output.expect_err("timeout"), ToolError::Timeout(50));
    }

    #[tokio::test]
    async fn honours_cancellation() {
        let cancel = CancellationToken::new();
        let ctx = ToolContext { run_id: "r".into(), call_id: "c".into(), cancel: cancel.clone() };
        let handle = tokio::spawn(async move { DefaultExecutor.execute(slow(10_000), json!({}), ctx).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
        let e = handle.await.expect("join");
        assert_eq!(e.output.expect_err("cancel"), ToolError::Cancelled);
        assert!(e.duration < Duration::from_secs(2));
    }
}
