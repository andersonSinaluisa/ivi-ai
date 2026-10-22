//! Executor: concurrencia global, resource locks, timeouts, cancelación, aislamiento de pánicos y
//! verificación separada de la ejecución (ADR-007).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::descriptor::{ToolDescriptor, VerificationMode};
use crate::error::{ErrorCode, ToolError};
use crate::locks::{LockManager, LockSet, ResourceLock};
use crate::result::{
    ErrorSection, ExecutionSection, ExecutionStatus, Metadata, Outcome, SCHEMA, ToolResult, VerificationSection,
    VerificationStatus,
};
use crate::tool::{ExecContext, ExecutionOutcome, Tool, VerificationReport};

/// Gracia tras el timeout antes de considerar la tool atascada (`TOOL_STUCK`).
pub const STUCK_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub struct ExecutorConfig {
    pub max_concurrent: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self { max_concurrent: 4 }
    }
}

/// Avisos de cambio de fase para que el llamante persista el estado local.
#[derive(Clone, Default)]
pub struct RunHooks {
    /// Se invoca al entrar en la fase de verificación (estado local `Verifying`).
    pub on_verifying: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for RunHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunHooks").field("on_verifying", &self.on_verifying.is_some()).finish()
    }
}

/// Metadatos que se copian al `ToolResult`.
#[derive(Debug, Clone)]
pub struct RunMetadata {
    pub risk_level: String,
    pub decision: String,
    pub invoked_by: String,
}

#[derive(Debug)]
pub struct Executor {
    locks: LockManager,
    slots: Arc<Semaphore>,
    running: Arc<AtomicUsize>,
    stuck: Arc<AtomicUsize>,
}

/// Permiso de ejecución: plaza de concurrencia + locks. Se libera cuando la ejecución termina de verdad.
#[derive(Debug)]
pub struct Admission {
    _permit: OwnedSemaphorePermit,
    _locks: LockSet,
}

/// Timeout efectivo = min(maxTimeout de la tool, maxDurationSeconds del Cloud, timeout pedido o default).
#[must_use]
pub fn effective_timeout(
    desc: &ToolDescriptor,
    max_duration: Option<Duration>,
    requested: Option<Duration>,
) -> Duration {
    let wanted = requested.unwrap_or(desc.timeouts.default);
    let mut timeout = wanted.min(desc.timeouts.max);
    if let Some(cloud) = max_duration {
        timeout = timeout.min(cloud);
    }
    timeout
}

impl Executor {
    #[must_use]
    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            locks: LockManager::default(),
            slots: Arc::new(Semaphore::new(config.max_concurrent.max(1))),
            running: Arc::new(AtomicUsize::new(0)),
            stuck: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Ejecuciones en curso (para `busy` en presencia y health).
    #[must_use]
    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// Tools que no terminaron tras su timeout + gracia.
    #[must_use]
    pub fn stuck(&self) -> usize {
        self.stuck.load(Ordering::SeqCst)
    }

    /// Espera plaza y locks. El tiempo en cola cuenta para el timeout efectivo.
    ///
    /// # Errors
    /// `TOOL_TIMEOUT` si vence el deadline esperando; `CANCELLED` si se cancela.
    pub async fn admit(
        &self,
        resources: Vec<ResourceLock>,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<Admission, ToolError> {
        let wait = async {
            let permit = self
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ToolError::new(ErrorCode::SystemError, "executor closed"))?;
            let locks = self.locks.acquire(resources).await;
            Ok::<_, ToolError>(Admission { _permit: permit, _locks: locks })
        };
        tokio::select! {
            () = cancel.cancelled() => Err(ToolError::new(ErrorCode::Cancelled, "cancelled while waiting for resources")),
            r = tokio::time::timeout_at(deadline, wait) => r.unwrap_or_else(|_| Err(ToolError::new(ErrorCode::ToolTimeout, "timed out waiting for resources"))),
        }
    }

    /// Ejecuta y verifica. Nunca devuelve éxito sin verificación `PASSED` (o `NOT_APPLICABLE`).
    pub async fn run(
        &self,
        admission: Admission,
        tool: Arc<dyn Tool>,
        input: Value,
        ctx: ExecContext,
        meta: RunMetadata,
        hooks: RunHooks,
    ) -> ToolResult {
        let desc = tool.descriptor().clone();
        let started_at = Utc::now();
        let t0 = Instant::now();
        self.running.fetch_add(1, Ordering::SeqCst);
        let running = self.running.clone();

        let inner_tool = tool.clone();
        let inner_input = input.clone();
        let inner_ctx = ctx.clone();
        let verification = desc.verification;
        let mut handle = tokio::spawn(async move {
            // La admisión vive dentro de la tarea: los locks se liberan cuando la tool termina de verdad.
            let _admission = admission;
            let exec_start = Instant::now();
            let exec = inner_tool.execute(&inner_input, &inner_ctx).await;
            let exec_ms = elapsed_ms(exec_start);
            let verify_start = Instant::now();
            let report = match &exec {
                Ok(outcome) if verification.mode == VerificationMode::Required => {
                    if let Some(on_verifying) = &hooks.on_verifying {
                        on_verifying();
                    }
                    // La cancelación no interrumpe la verificación (solo observa): token propio.
                    let verify_ctx = ExecContext { cancel: CancellationToken::new(), ..inner_ctx.clone() };
                    // La tool gestiona su propio presupuesto de sondeo; este timeout es solo una
                    // red de seguridad, por eso se le da un margen extra.
                    let grace = Duration::from_millis(500);
                    let budget = (verification.timeout + grace)
                        .min(inner_ctx.deadline.saturating_duration_since(Instant::now()) + grace);
                    tokio::time::timeout(budget, inner_tool.verify(&inner_input, outcome, &verify_ctx))
                        .await
                        .unwrap_or_else(|_| {
                            VerificationReport::inconclusive(Vec::new(), "verification did not complete in time")
                        })
                }
                Ok(_) => VerificationReport::not_applicable(),
                Err(_) => VerificationReport {
                    status: VerificationStatus::Skipped,
                    checks: Vec::new(),
                    message: None,
                    output_patch: None,
                },
            };
            (exec, exec_ms, report, elapsed_ms(verify_start))
        });

        let joined = tokio::time::timeout_at(ctx.deadline, &mut handle).await;
        let (exec, exec_ms, report, verify_ms) = match joined {
            Ok(Ok(parts)) => parts,
            Ok(Err(join_error)) => {
                tracing::error!(tool = desc.name, panic = join_error.is_panic(), "tool task aborted");
                (Err(ToolError::new(ErrorCode::SystemError, "internal error in tool")), elapsed_ms(t0), skipped(), 0)
            }
            Err(_) => {
                // Timeout: se pide la parada cooperativa y se vigila si la tool queda atascada.
                ctx.cancel.cancel();
                self.watch_stuck(handle, desc.name);
                let error = ToolError::new(ErrorCode::ToolTimeout, format!("{} exceeded its timeout", desc.name));
                (Err(error), elapsed_ms(t0), skipped(), 0)
            }
        };
        running.fetch_sub(1, Ordering::SeqCst);
        assemble(&desc, ctx.execution_id, started_at, t0, exec, exec_ms, report, verify_ms, meta)
    }

    fn watch_stuck(&self, handle: tokio::task::JoinHandle<impl Send + 'static>, tool: &'static str) {
        let stuck = self.stuck.clone();
        tokio::spawn(async move {
            if tokio::time::timeout(STUCK_GRACE, handle).await.is_err() {
                stuck.fetch_add(1, Ordering::SeqCst);
                tracing::error!(tool, "TOOL_STUCK: tool did not stop after timeout + grace");
            }
        });
    }
}

fn skipped() -> VerificationReport {
    VerificationReport { status: VerificationStatus::Skipped, checks: Vec::new(), message: None, output_patch: None }
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    desc: &ToolDescriptor,
    execution_id: uuid::Uuid,
    started_at: chrono::DateTime<Utc>,
    t0: Instant,
    exec: Result<ExecutionOutcome, ToolError>,
    exec_ms: u64,
    report: VerificationReport,
    verify_ms: u64,
    meta: RunMetadata,
) -> ToolResult {
    let (exec_status, details, mut output, mut error) = match exec {
        Ok(outcome) => (ExecutionStatus::Succeeded, outcome.details, Some(outcome.output), None),
        Err(e) => {
            let status = match e.code {
                ErrorCode::Cancelled => ExecutionStatus::Cancelled,
                ErrorCode::ToolTimeout => ExecutionStatus::TimedOut,
                _ => ExecutionStatus::Failed,
            };
            (status, None, None, Some(e))
        }
    };

    if let (Some(out), Some(patch)) = (output.as_mut(), report.output_patch.clone()) {
        merge(out, patch);
    }

    if error.is_none() {
        error = match report.status {
            VerificationStatus::Failed => Some(ToolError::new(
                ErrorCode::VerificationFailed,
                report.message.clone().unwrap_or_else(|| "verification failed".into()),
            )),
            VerificationStatus::Inconclusive => Some(ToolError::new(
                ErrorCode::VerificationInconclusive,
                report.message.clone().unwrap_or_else(|| "verification inconclusive".into()),
            )),
            _ => None,
        };
    }

    let mut outcome = ToolResult::derive_outcome(exec_status, report.status);
    if outcome == Outcome::Succeeded
        && let Some(out) = &output
    {
        let size = serde_json::to_vec(out).map_or(usize::MAX, |b| b.len());
        if size > desc.max_output_bytes {
            error =
                Some(ToolError::new(ErrorCode::OutputTooLarge, format!("output of {size} bytes exceeds the limit")));
            outcome = Outcome::Failed;
            output = None;
        }
    }

    ToolResult {
        schema: SCHEMA.to_owned(),
        tool: desc.name.to_owned(),
        tool_version: desc.version.to_owned(),
        execution_id,
        outcome,
        started_at,
        finished_at: Utc::now(),
        duration_ms: elapsed_ms(t0),
        execution: ExecutionSection { status: exec_status, duration_ms: exec_ms, details },
        verification: VerificationSection {
            status: report.status,
            method: if report.status == VerificationStatus::NotApplicable {
                "none".to_owned()
            } else {
                desc.verification.method.to_owned()
            },
            duration_ms: verify_ms,
            checks: report.checks,
        },
        output,
        truncated: false,
        error: error.as_ref().map(ErrorSection::from),
        metadata: Some(Metadata { risk_level: meta.risk_level, decision: meta.decision, invoked_by: meta.invoked_by }),
    }
}

fn merge(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                t.insert(k, v);
            }
        }
        (t, p) => *t = p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationSpec};
    use crate::result::Check;
    use crate::risk::RiskLevel;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicU32;

    /// Tool de prueba configurable (equivalente a la `noop` de HU-WIN-022, solo en tests).
    struct Noop {
        desc: ToolDescriptor,
        delay: Duration,
        fail: Option<ErrorCode>,
        verify_after_polls: Option<u32>,
        panic: bool,
        executions: AtomicU32,
    }

    impl Noop {
        fn new(risk: RiskLevel) -> Self {
            let verification = if risk >= RiskLevel::R2 {
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "noop_condition",
                    timeout: Duration::from_millis(500),
                    poll_interval: Duration::from_millis(10),
                }
            } else {
                VerificationSpec::not_applicable()
            };
            Self {
                desc: ToolDescriptor {
                    name: "noop",
                    version: "1.0.0",
                    description: "test tool",
                    risk_level: risk,
                    required_permissions: vec!["test.noop"],
                    input_schema: serde_json::json!({}),
                    output_schema: serde_json::json!({}),
                    timeouts: Timeouts { default: Duration::from_secs(1), max: Duration::from_secs(2) },
                    cancellation: CancellationMode::Cooperative,
                    verification,
                    sensitivity: Sensitivity::None,
                    max_output_bytes: 1024,
                    requires_interactive_session: false,
                    idempotent_effect: true,
                },
                delay: Duration::ZERO,
                fail: None,
                verify_after_polls: Some(0),
                panic: false,
                executions: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl Tool for Noop {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.desc
        }
        fn validate(&self, input: &Value) -> Result<Value, ToolError> {
            Ok(input.clone())
        }
        async fn execute(&self, _input: &Value, ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "boom");
            ctx.sleep(self.delay).await?;
            if let Some(code) = self.fail {
                return Err(ToolError::new(code, "simulated failure"));
            }
            Ok(ExecutionOutcome { output: serde_json::json!({"ok": true}), ..Default::default() })
        }
        async fn verify(&self, _i: &Value, _o: &ExecutionOutcome, ctx: &ExecContext) -> VerificationReport {
            let Some(needed) = self.verify_after_polls else {
                tokio::time::sleep(Duration::from_secs(10)).await;
                return VerificationReport::inconclusive(vec![], "never");
            };
            let mut polls = 0;
            loop {
                if polls >= needed {
                    return VerificationReport::passed(vec![Check::new("condition", true, None)]);
                }
                polls += 1;
                if ctx.sleep(self.desc.verification.poll_interval).await.is_err() {
                    return VerificationReport::failed(vec![], "cancelled");
                }
            }
        }
    }

    fn meta() -> RunMetadata {
        RunMetadata { risk_level: "R2".into(), decision: "APPROVED".into(), invoked_by: "local".into() }
    }

    async fn run(tool: Noop, timeout: Duration) -> (ToolResult, Arc<Noop>) {
        let exec = Executor::new(ExecutorConfig::default());
        let tool = Arc::new(tool);
        let ctx = ExecContext {
            execution_id: uuid::Uuid::now_v7(),
            deadline: Instant::now() + timeout,
            cancel: CancellationToken::new(),
        };
        let admission = exec.admit(vec![ResourceLock::app("x")], ctx.deadline, &ctx.cancel).await.expect("admitted");
        let result = exec.run(admission, tool.clone(), Value::Null, ctx, meta(), RunHooks::default()).await;
        (result, tool)
    }

    #[tokio::test]
    async fn success_is_reported_only_after_verification_passes() {
        let mut tool = Noop::new(RiskLevel::R2);
        tool.verify_after_polls = Some(3);
        let (result, _) = run(tool, Duration::from_secs(1)).await;
        assert_eq!(result.outcome, Outcome::Succeeded);
        assert_eq!(result.verification.status, VerificationStatus::Passed);
        assert_eq!(result.verification.method, "noop_condition");
        assert_eq!(result.schema, SCHEMA);
    }

    #[tokio::test]
    async fn verification_that_never_passes_is_a_failure_not_a_success() {
        let mut tool = Noop::new(RiskLevel::R2);
        tool.verify_after_polls = None;
        let (result, _) = run(tool, Duration::from_secs(2)).await;
        assert_eq!(result.outcome, Outcome::Failed);
        assert_eq!(result.execution.status, ExecutionStatus::Succeeded);
        assert_eq!(result.verification.status, VerificationStatus::Inconclusive);
        assert_eq!(result.error.map(|e| e.code), Some(ErrorCode::VerificationInconclusive));
    }

    #[tokio::test]
    async fn observation_tools_do_not_need_verification() {
        let (result, _) = run(Noop::new(RiskLevel::R0), Duration::from_secs(1)).await;
        assert_eq!(result.outcome, Outcome::Succeeded);
        assert_eq!(result.verification.status, VerificationStatus::NotApplicable);
    }

    #[tokio::test]
    async fn execution_failure_skips_verification() {
        let mut tool = Noop::new(RiskLevel::R2);
        tool.fail = Some(ErrorCode::AppElevated);
        let (result, _) = run(tool, Duration::from_secs(1)).await;
        assert_eq!(result.outcome, Outcome::Failed);
        assert_eq!(result.verification.status, VerificationStatus::Skipped);
        assert_eq!(result.error.map(|e| e.code), Some(ErrorCode::AppElevated));
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn timeout_is_reported_as_tool_timeout() {
        let mut tool = Noop::new(RiskLevel::R2);
        tool.delay = Duration::from_secs(5);
        let (result, _) = run(tool, Duration::from_millis(100)).await;
        assert_eq!(result.execution.status, ExecutionStatus::TimedOut);
        assert_eq!(result.error.map(|e| e.code), Some(ErrorCode::ToolTimeout));
    }

    #[tokio::test]
    async fn cancellation_is_reported_as_cancelled() {
        let exec = Executor::new(ExecutorConfig::default());
        let mut tool = Noop::new(RiskLevel::R2);
        tool.delay = Duration::from_secs(5);
        let ctx = ExecContext {
            execution_id: uuid::Uuid::now_v7(),
            deadline: Instant::now() + Duration::from_secs(2),
            cancel: CancellationToken::new(),
        };
        let admission = exec.admit(vec![], ctx.deadline, &ctx.cancel).await.expect("admitted");
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let result = exec.run(admission, Arc::new(tool), Value::Null, ctx, meta(), RunHooks::default()).await;
        assert_eq!(result.outcome, Outcome::Cancelled);
        assert_eq!(result.execution.status, ExecutionStatus::Cancelled);
    }

    #[tokio::test]
    async fn a_panicking_tool_does_not_bring_down_the_executor() {
        let mut tool = Noop::new(RiskLevel::R2);
        tool.panic = true;
        let (result, _) = run(tool, Duration::from_secs(1)).await;
        assert_eq!(result.outcome, Outcome::Failed);
        assert_eq!(result.error.map(|e| e.code), Some(ErrorCode::SystemError));
    }

    #[tokio::test]
    async fn effective_timeout_is_the_minimum() {
        let tool = Noop::new(RiskLevel::R2);
        let d = &tool.desc;
        assert_eq!(effective_timeout(d, None, None), Duration::from_secs(1));
        assert_eq!(effective_timeout(d, None, Some(Duration::from_secs(60))), Duration::from_secs(2));
        assert_eq!(effective_timeout(d, Some(Duration::from_millis(300)), None), Duration::from_millis(300));
    }

    #[tokio::test]
    async fn admission_times_out_when_resources_stay_busy() {
        let exec = Executor::new(ExecutorConfig { max_concurrent: 1 });
        let cancel = CancellationToken::new();
        let first = exec.admit(vec![], Instant::now() + Duration::from_secs(1), &cancel).await.expect("first");
        let second = exec.admit(vec![], Instant::now() + Duration::from_millis(50), &cancel).await;
        assert_eq!(second.map(|_| ()).map_err(|e| e.code), Err(ErrorCode::ToolTimeout));
        drop(first);
    }
}
