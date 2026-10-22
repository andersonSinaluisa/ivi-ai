//! Sistema de tools del Windows Agent (docs/03-tools).
//!
//! Independiente del transporte: nada aquí conoce MQTT ni el Cloud. Lo usan igual el frontend
//! de tareas del Cloud y la futura Local Tool API del Agent Core (ADR-003, ADR-014).

pub mod descriptor;
pub mod error;
pub mod executor;
pub mod locks;
pub mod platform;
pub mod registry;
pub mod result;
pub mod risk;
pub mod tool;

pub use descriptor::{CancellationMode, Sensitivity, Timeouts, ToolDescriptor, VerificationMode, VerificationSpec};
pub use error::{ErrorCategory, ErrorCode, ToolError};
pub use executor::{Admission, Executor, ExecutorConfig, RunHooks, RunMetadata};
pub use locks::{LockManager, LockMode, ResourceLock};
pub use registry::{Resolved, ToolRegistry};
pub use result::{Check, ExecutionStatus, Outcome, ToolResult, VerificationStatus};
pub use risk::RiskLevel;
pub use tool::{ExecContext, ExecutionOutcome, Tool, VerificationReport};
