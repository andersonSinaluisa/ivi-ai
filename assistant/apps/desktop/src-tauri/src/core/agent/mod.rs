//! AgentOrchestrator y sus componentes de fase.

pub mod approval;
pub mod context;
pub mod orchestrator;
pub mod personality;
pub mod sink;

pub use approval::ApprovalGate;
pub use context::{ContextBuilder, DefaultObserver, Observation, Observer};
pub use orchestrator::{AgentConfig, AgentDeps, AgentOrchestrator, AgentRequest, RunOutcome, RunStatus};
pub use personality::Personality;
pub use sink::{EventSink, FnSink};
