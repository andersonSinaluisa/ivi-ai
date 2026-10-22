//! AUTHORIZE (parte humana): pide confirmación a la UI y espera la respuesta.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::sink::EventSink;
use crate::domain::{AgentEvent, RiskClass};

pub struct ApprovalRequest<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub tool: &'a str,
    pub arguments: &'a Value,
    pub risk: RiskClass,
    pub reason: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOutcome {
    pub approved: bool,
    pub reason: Option<String>,
}

pub struct ApprovalGate {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    timeout: Duration,
}

impl ApprovalGate {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { pending: Mutex::new(HashMap::new()), timeout }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<bool>>> {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Emite `approval_required` y espera la decisión. Sin respuesta en el plazo → denegada.
    pub async fn request(
        &self,
        req: ApprovalRequest<'_>,
        sink: &dyn EventSink,
        cancel: &CancellationToken,
    ) -> ApprovalOutcome {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending().insert(approval_id.clone(), tx);
        sink.emit(AgentEvent::ApprovalRequired {
            run_id: req.run_id.to_owned(),
            approval_id: approval_id.clone(),
            call_id: req.call_id.to_owned(),
            tool: req.tool.to_owned(),
            arguments: req.arguments.clone(),
            risk: req.risk,
            reason: req.reason.to_owned(),
        });

        let outcome = tokio::select! {
            () = cancel.cancelled() => ApprovalOutcome { approved: false, reason: Some("tarea cancelada".into()) },
            r = tokio::time::timeout(self.timeout, rx) => match r {
                Ok(Ok(true)) => ApprovalOutcome { approved: true, reason: None },
                Ok(Ok(false)) => ApprovalOutcome { approved: false, reason: Some("el usuario la rechazó".into()) },
                Ok(Err(_)) => ApprovalOutcome { approved: false, reason: Some("solicitud descartada".into()) },
                Err(_) => ApprovalOutcome { approved: false, reason: Some("sin respuesta del usuario a tiempo".into()) },
            },
        };
        self.pending().remove(&approval_id);
        sink.emit(AgentEvent::ApprovalResolved {
            run_id: req.run_id.to_owned(),
            approval_id,
            approved: outcome.approved,
            reason: outcome.reason.clone(),
        });
        outcome
    }

    /// Respuesta del usuario. Devuelve `false` si la solicitud ya no existe.
    pub fn resolve(&self, approval_id: &str, approved: bool) -> bool {
        self.pending().remove(approval_id).is_some_and(|tx| tx.send(approved).is_ok())
    }
}
