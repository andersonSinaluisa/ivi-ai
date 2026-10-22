//! Task pipeline (docs/01-architecture/execution-pipeline.md).
//!
//! Orden: deserializar → envelope → canal autenticado → idempotencia → expiración → resolución de
//! tool → validación de input → riesgo → permisos → aprobación → persistir + ack → programar →
//! ejecutar → verificar → persistir resultado → publicar → *settle*.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::Utc;
use jarvis_policy::{Authenticity, DecisionKind, PolicyState, Principal};
use jarvis_protocol::codes::{cancel_reasons, message_types as mt};
use jarvis_protocol::messages::{
    ApprovalDecision, ErrorInfo, TaskApprovalRequired, TaskCancel, TaskCancelled, TaskCompleted, TaskDispatch,
    TaskFailed, TaskRef, TaskRejected, TaskStarted, TaskUpdateAck,
};
use jarvis_protocol::signature;
use jarvis_protocol::{Channel, Envelope};
use jarvis_state::store::FinishExecution;
use jarvis_state::{ApprovalState, LocalState, NewTask, OutboxMessage, Store, TaskRow};
use jarvis_tooling::executor::effective_timeout;
use jarvis_tooling::{
    CancellationMode, ErrorCode, ExecContext, Executor, Outcome, Resolved, RunHooks, RunMetadata, ToolError,
    ToolRegistry, ToolResult,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::health::{Health, Severity, codes};

/// Margen de la guardia de reinicio de la base local: el reloj del sistema no es monótono
/// (puede retroceder unos milisegundos entre dos lecturas), y una tarea recién creada nunca es
/// anterior a la base por más de esto. Las tareas realmente viejas lo son por minutos o días.
pub const STATE_RESET_TOLERANCE: chrono::Duration = chrono::Duration::seconds(60);

/// Margen de frescura de una decisión de aprobación firmada.
const DECISION_MAX_AGE_SECONDS: i64 = 300;

/// Publica mensajes ya persistidos en el outbox. Implementado por el Cloud Link (y por dobles en tests).
#[async_trait::async_trait]
pub trait Publisher: Send + Sync {
    /// `true` si el mensaje salió; `false` si no hay conexión (se reintentará desde el outbox).
    async fn publish(&self, channel: Channel, payload: Vec<u8>) -> bool;
}

/// Avisa al usuario de que hay algo esperando su decisión (icono del área de notificación).
pub trait ApprovalNotifier: Send + Sync {
    fn approval_required(&self, tool: &str, risk: &str, task_id: Uuid);
}

pub struct Agent {
    pub device_id: Uuid,
    pub store: Arc<Store>,
    pub registry: Arc<ToolRegistry>,
    pub executor: Arc<Executor>,
    policy: Mutex<Arc<PolicyState>>,
    pub health: Arc<Health>,
    pub publisher: Arc<dyn Publisher>,
    running: Mutex<HashMap<Uuid, CancellationToken>>,
    /// En modo seguro se publican los resultados pendientes pero no se aceptan tareas nuevas.
    safe_mode: std::sync::atomic::AtomicBool,
    notifier: Mutex<Option<Arc<dyn ApprovalNotifier>>>,
}

impl Agent {
    #[must_use]
    pub fn new(
        device_id: Uuid,
        store: Arc<Store>,
        registry: Arc<ToolRegistry>,
        executor: Arc<Executor>,
        policy: Arc<PolicyState>,
        health: Arc<Health>,
        publisher: Arc<dyn Publisher>,
    ) -> Arc<Self> {
        Arc::new(Self {
            device_id,
            store,
            registry,
            executor,
            policy: Mutex::new(policy),
            health,
            publisher,
            running: Mutex::new(HashMap::new()),
            safe_mode: std::sync::atomic::AtomicBool::new(false),
            notifier: Mutex::new(None),
        })
    }

    /// Registra quién avisa al usuario de las aprobaciones pendientes.
    pub fn set_notifier(&self, notifier: Arc<dyn ApprovalNotifier>) {
        *self.notifier.lock().unwrap_or_else(PoisonError::into_inner) = Some(notifier);
    }

    /// Política vigente (se puede sustituir en caliente con `policy reload`).
    #[must_use]
    pub fn policy(&self) -> Arc<PolicyState> {
        self.policy.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Sustituye la política. Una política inválida nunca llega aquí: el llamante conserva la anterior.
    pub fn set_policy(&self, policy: Arc<PolicyState>) {
        *self.policy.lock().unwrap_or_else(PoisonError::into_inner) = policy;
    }

    /// Activa o levanta el modo seguro (crash loop, HU-WIN-018).
    pub fn set_safe_mode(&self, enabled: bool) {
        self.safe_mode.store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    #[must_use]
    pub fn safe_mode(&self) -> bool {
        self.safe_mode.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn envelope<T: Serialize>(&self, kind: &str, data: &T, correlation: Option<Uuid>) -> Option<OutboxMessage> {
        match Envelope::create(kind, self.device_id, data, correlation, None) {
            Ok(envelope) => envelope
                .to_bytes()
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|json| OutboxMessage::new(kind, json)),
            Err(error) => {
                tracing::error!(%error, kind, "cannot build outgoing message");
                None
            }
        }
    }

    /// Publica todo lo pendiente del outbox, en orden.
    pub async fn flush_outbox(&self) {
        let pending = match self.store.pending_outbox() {
            Ok(pending) => pending,
            Err(error) => {
                tracing::error!(%error, "cannot read outbox");
                self.health.raise(codes::DB_ERROR, Severity::Error, error.to_string());
                return;
            }
        };
        for (seq, payload) in pending {
            if self.publisher.publish(Channel::Tasks, payload.into_bytes()).await {
                if let Err(error) = self.store.mark_published(seq) {
                    tracing::error!(%error, seq, "cannot mark outbox message as published");
                }
            } else {
                break; // Sin conexión: se conserva el orden y se reintenta al reconectar.
            }
        }
    }

    /// Reenvía el último estado de cada tarea no *settled* (tras reconectar y periódicamente, W6).
    pub async fn resend_unsettled(&self) {
        let Ok(candidates) = self.store.resend_candidates() else { return };
        for (_, payload) in candidates {
            if !self.publisher.publish(Channel::Tasks, payload.into_bytes()).await {
                break;
            }
        }
    }

    /// Entrada del Cloud Link: un mensaje ya validado como `down/*` de este device.
    pub async fn handle_envelope(self: &Arc<Self>, channel: Channel, envelope: Envelope) {
        if channel != Channel::Tasks {
            tracing::debug!(kind = envelope.kind, "message ignored (not a task channel)");
            return;
        }
        match envelope.kind.as_str() {
            mt::TASK_DISPATCH => match envelope.data_as::<TaskDispatch>() {
                Ok(dispatch) => self.handle_dispatch(&envelope, dispatch).await,
                Err(error) => {
                    // `taskId` legible con `data` inválido: se puede responder.
                    if let Some(task_id) = envelope.task_id_hint() {
                        self.reject_unknown(task_id, envelope.correlation_id, &ToolError::invalid(error.to_string()))
                            .await;
                    } else {
                        tracing::warn!(%error, "invalid task.dispatch discarded");
                    }
                }
            },
            mt::TASK_CANCEL => match envelope.data_as::<TaskCancel>() {
                Ok(cancel) => self.handle_cancel(&envelope, cancel).await,
                Err(error) => tracing::warn!(%error, "invalid task.cancel discarded"),
            },
            mt::TASK_UPDATE_ACK => match envelope.data_as::<TaskUpdateAck>() {
                Ok(ack) => self.handle_update_ack(ack).await,
                Err(error) => tracing::warn!(%error, "invalid task.update.ack discarded"),
            },
            mt::TASK_APPROVAL_DECISION => match envelope.data_as::<ApprovalDecision>() {
                Ok(decision) => self.handle_approval_decision(decision).await,
                // Una decisión mal formada se ignora: la aprobación sigue pendiente.
                Err(error) => tracing::warn!(%error, "invalid task.approval_decision discarded"),
            },
            other => tracing::debug!(kind = other, "unsupported message type ignored"),
        }
    }

    #[tracing::instrument(skip_all, fields(task_id = %dispatch.task_id, task_type = %dispatch.task_type, attempt = dispatch.attempt))]
    async fn handle_dispatch(self: &Arc<Self>, envelope: &Envelope, dispatch: TaskDispatch) {
        // Etapa 4: idempotencia antes que expiración. Una tarea conocida nunca se ejecuta otra vez.
        match self.store.get_task(dispatch.task_id) {
            Ok(Some(known)) => {
                tracing::info!(state = known.state.as_str(), "duplicate dispatch: resending last state");
                if known.payload_hash != jarvis_state::payload_hash(&dispatch.payload) {
                    tracing::warn!(
                        stored = known.payload_hash,
                        received = jarvis_state::payload_hash(&dispatch.payload),
                        "same taskId with a different payload; the first one prevails"
                    );
                }
                if let Ok(Some(last)) = self.store.last_message(dispatch.task_id) {
                    self.publisher.publish(Channel::Tasks, last.into_bytes()).await;
                }
                return;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "cannot read the local state; not acknowledging");
                self.health.raise(codes::DB_ERROR, Severity::Error, error.to_string());
                return;
            }
        }

        let correlation = envelope.correlation_id;
        let now = self.health.now();

        // Modo seguro: la tarea anterior pudo tumbar el proceso; no se aceptan nuevas hasta que el
        // owner lo reconozca (lo pendiente sí se sigue publicando).
        if self.safe_mode() {
            self.reject_unknown(
                dispatch.task_id,
                correlation,
                &ToolError::new(ErrorCode::NotPermitted, "agent in safe mode after repeated crashes"),
            )
            .await;
            return;
        }

        // Guardia de reinicio de la base local: una tarea anterior a la BD no se ejecuta.
        if dispatch.created_at < self.store.db_created_at() - STATE_RESET_TOLERANCE {
            self.reject_unknown(
                dispatch.task_id,
                correlation,
                &ToolError::new(ErrorCode::StateReset, "the local state was reset; resubmit the task"),
            )
            .await;
            return;
        }
        if now > dispatch.expires_at {
            self.reject_unknown(
                dispatch.task_id,
                correlation,
                &ToolError::new(ErrorCode::ExpiredOnArrival, "the task expired before arriving"),
            )
            .await;
            return;
        }

        // Etapa 6: resolución de tool (incluye alias y tools deshabilitadas).
        let Some(resolved) = self.registry.resolve_task_type(&dispatch.task_type, &dispatch.payload) else {
            self.reject_unknown(
                dispatch.task_id,
                correlation,
                &ToolError::new(
                    ErrorCode::UnsupportedTaskType,
                    format!("{} is not supported by this agent", dispatch.task_type),
                ),
            )
            .await;
            return;
        };

        // Etapa 7: validación estricta del input.
        let input = match resolved.tool.validate(&resolved.input) {
            Ok(input) => input,
            Err(error) => {
                self.reject_unknown(dispatch.task_id, correlation, &error).await;
                return;
            }
        };

        // Etapas 8-9: riesgo efectivo y permisos.
        let Some(principal) = Principal::from_cloud(&dispatch.requested_by.kind, dispatch.requested_by.id) else {
            self.reject_unknown(
                dispatch.task_id,
                correlation,
                &ToolError::new(ErrorCode::NotPermitted, "unknown requester type"),
            )
            .await;
            return;
        };
        // Etapa 3: autenticidad del originador. Sin firma la identidad es solo afirmada (C10);
        // con firma inválida la tarea se rechaza aquí, antes de mirar permisos.
        let authenticity = match self.authenticate(&dispatch, &principal) {
            Ok(authenticity) => authenticity,
            Err(error) => {
                tracing::warn!(%error, principal = %principal, "task signature rejected");
                self.reject_unknown(dispatch.task_id, correlation, &error).await;
                return;
            }
        };

        let tool_name = resolved.tool.descriptor().name;
        let risk = resolved.tool.risk(&input);
        let permissions = resolved.tool.required_permissions(&input);
        let decision =
            jarvis_policy::evaluate(&self.policy(), &principal, tool_name, &input, risk, &permissions, &authenticity);
        tracing::info!(
            tool = tool_name,
            %risk,
            principal = %principal,
            decision = decision.kind.as_str(),
            reason = decision.reason_code,
            "policy decision"
        );

        let sensitive = resolved.tool.descriptor().sensitivity != jarvis_tooling::Sensitivity::None;
        let mut task = NewTask {
            task_id: dispatch.task_id,
            correlation_id: correlation,
            task_type: dispatch.task_type.clone(),
            tool: Some(tool_name.to_owned()),
            principal: principal.policy_id(),
            payload: dispatch.payload.clone(),
            state: LocalState::Accepted,
            risk_level: Some(risk.to_string()),
            decision: Some(decision.kind.as_str().to_owned()),
            decision_reason: Some(decision.reason_code.to_owned()),
            created_at: dispatch.created_at,
            expires_at: dispatch.expires_at,
            max_duration_seconds: dispatch.max_duration_seconds,
            sensitive,
            // Lo que se probó al recibir: la re-evaluación previa a ejecutar lo vuelve a usar.
            signed_by: match &authenticity {
                Authenticity::Signed(key_id) => Some(key_id.clone()),
                Authenticity::Claimed | Authenticity::Local => None,
            },
        };

        match decision.kind {
            DecisionKind::Denied => {
                task.state = LocalState::Rejected;
                let error = ToolError::new(ErrorCode::NotPermitted, decision.message.clone());
                let Some(message) = self.rejected_message(dispatch.task_id, correlation, &error) else { return };
                self.persist_and_flush(&task, &[message]).await;
            }
            DecisionKind::RequiresApproval => {
                task.state = LocalState::AwaitingApproval;
                let mut messages = Vec::new();
                if let Some(ack) = self.envelope(mt::TASK_ACK, &TaskRef { task_id: dispatch.task_id }, correlation) {
                    messages.push(ack);
                }
                let reason = if decision.message.is_empty() {
                    format!("{tool_name} requires local approval")
                } else {
                    decision.message.clone()
                };
                if let Some(required) = self.envelope(
                    mt::TASK_APPROVAL_REQUIRED,
                    &TaskApprovalRequired { task_id: dispatch.task_id, reason },
                    correlation,
                ) {
                    messages.push(required);
                }
                if !self.persist_and_flush(&task, &messages).await {
                    return;
                }
                let ttl = i64::try_from(self.policy().policy.risk.approval_ttl_seconds).unwrap_or(300);
                let expires = dispatch.expires_at.min(now + chrono::Duration::seconds(ttl));
                // El hash que se guarda es el del **payload de la tarea** en forma canónica, no el
                // del input ya mapeado: es lo único que un aprobador remoto puede reproducir para
                // firmar su decisión (el alias OPEN_CHROME, por ejemplo, cambia el input).
                if let Err(error) =
                    self.store.create_approval(dispatch.task_id, &signature::payload_hash(&dispatch.payload), expires)
                {
                    tracing::error!(%error, "cannot persist the approval");
                }
                let notifier = self.notifier.lock().unwrap_or_else(PoisonError::into_inner).clone();
                if let Some(notifier) = notifier {
                    notifier.approval_required(tool_name, &risk.to_string(), dispatch.task_id);
                }
            }
            DecisionKind::Approved => {
                if dispatch.cancel_requested {
                    task.state = LocalState::Cancelled;
                    let message = self.envelope(
                        mt::TASK_CANCELLED,
                        &TaskCancelled { task_id: dispatch.task_id, reason: cancel_reasons::USER_REQUEST.to_owned() },
                        correlation,
                    );
                    self.persist_and_flush(&task, message.as_slice()).await;
                    return;
                }
                let Some(ack) = self.envelope(mt::TASK_ACK, &TaskRef { task_id: dispatch.task_id }, correlation) else {
                    return;
                };
                if !self.persist_and_flush(&task, &[ack]).await {
                    return;
                }
                self.clone().spawn_execution(dispatch.task_id);
            }
        }
    }

    async fn persist_and_flush(&self, task: &NewTask, messages: &[OutboxMessage]) -> bool {
        // W5: persistir (estado + mensajes) antes de publicar nada.
        if let Err(error) = self.store.insert_task(task, messages) {
            tracing::error!(%error, "cannot persist the task; the Cloud will retry");
            self.health.raise(codes::DB_ERROR, Severity::Error, error.to_string());
            return false;
        }
        self.flush_outbox().await;
        true
    }

    fn rejected_message(&self, task_id: Uuid, correlation: Option<Uuid>, error: &ToolError) -> Option<OutboxMessage> {
        self.envelope(
            mt::TASK_REJECTED,
            &TaskRejected {
                task_id,
                error: ErrorInfo {
                    // Al cable solo van códigos del catálogo del Cloud (C5).
                    code: error.code.reject_code().to_owned(),
                    message: format!("{}: {}", error.code, error.message),
                    retryable: false,
                },
            },
            correlation,
        )
    }

    /// Rechazo de una tarea que aún no existía localmente (se registra para no re-evaluarla).
    async fn reject_unknown(&self, task_id: Uuid, correlation: Option<Uuid>, error: &ToolError) {
        tracing::info!(%task_id, code = %error.code, message = %error.message, "task rejected");
        let Some(message) = self.rejected_message(task_id, correlation, error) else { return };
        let now = self.health.now();
        let task = NewTask {
            task_id,
            correlation_id: correlation,
            task_type: "UNKNOWN".to_owned(),
            tool: None,
            principal: "unknown".to_owned(),
            payload: Value::Null,
            state: LocalState::Rejected,
            risk_level: None,
            decision: Some("DENIED".to_owned()),
            decision_reason: Some(error.code.as_str().to_owned()),
            created_at: now,
            expires_at: now,
            max_duration_seconds: None,
            sensitive: false,
            signed_by: None,
        };
        self.persist_and_flush(&task, &[message]).await;
    }

    async fn handle_update_ack(&self, ack: TaskUpdateAck) {
        match self.store.settle(ack.task_id) {
            Ok(sensitive) => {
                tracing::info!(task_id = %ack.task_id, status = ?ack.status, "task settled");
                if sensitive {
                    // Etapa 17: tras el ack se redacta lo sensible (se conservan hash y tamaño).
                    let redacted = json!({ "redacted": true, "reason": "sensitive tool output" }).to_string();
                    if let Err(error) = self.store.redact(ack.task_id, &redacted, "{}") {
                        tracing::error!(%error, "cannot redact the stored result");
                    }
                }
            }
            Err(error) => tracing::error!(%error, "cannot settle the task"),
        }
    }

    async fn handle_cancel(self: &Arc<Self>, envelope: &Envelope, cancel: TaskCancel) {
        let correlation = envelope.correlation_id;
        let known = self.store.get_task(cancel.task_id).ok().flatten();
        let Some(task) = known else {
            // Tarea desconocida: se registra como cancelada para rechazar un dispatch posterior (W8).
            let now = self.health.now();
            let message = self.envelope(
                mt::TASK_CANCELLED,
                &TaskCancelled { task_id: cancel.task_id, reason: cancel_reasons::USER_REQUEST.to_owned() },
                correlation,
            );
            let tombstone = NewTask {
                task_id: cancel.task_id,
                correlation_id: correlation,
                task_type: "UNKNOWN".to_owned(),
                tool: None,
                principal: "unknown".to_owned(),
                payload: Value::Null,
                state: LocalState::Cancelled,
                risk_level: None,
                decision: None,
                decision_reason: Some("CANCELLED_BEFORE_DISPATCH".to_owned()),
                created_at: now,
                expires_at: now,
                max_duration_seconds: None,
                sensitive: false,
                signed_by: None,
            };
            self.persist_and_flush(&tombstone, message.as_slice()).await;
            return;
        };

        if task.state.is_terminal() {
            if let Ok(Some(last)) = self.store.last_message(task.task_id) {
                self.publisher.publish(Channel::Tasks, last.into_bytes()).await;
            }
            return;
        }

        let token = self.running.lock().unwrap_or_else(PoisonError::into_inner).get(&task.task_id).cloned();
        if let Some(token) = token {
            let cancellable = self
                .registry
                .resolve_task_type(&task.task_type, &task.payload)
                .map(|r| r.tool.descriptor().cancellation)
                .unwrap_or(CancellationMode::Cooperative);
            if cancellable == CancellationMode::None {
                // C12: se informa y se reporta el estado terminal real cuando llegue.
                let message = self.envelope(
                    mt::TASK_PROGRESS,
                    &jarvis_protocol::messages::TaskProgress {
                        task_id: task.task_id,
                        percent: None,
                        message: Some("cancellation not supported by tool".to_owned()),
                    },
                    task.correlation_id,
                );
                if let Some(message) = message {
                    self.publisher.publish(Channel::Tasks, message.envelope_json.into_bytes()).await;
                }
                return;
            }
            token.cancel();
            return;
        }

        // En cola o esperando aprobación: se cancela sin ejecutar.
        let _ = self.store.decide_approval(task.task_id, ApprovalState::Denied, "cloud:cancel");
        self.terminate_without_execution(&task, cancel_reasons::USER_REQUEST).await;
    }

    async fn terminate_without_execution(&self, task: &TaskRow, reason: &str) {
        let message = self.envelope(
            mt::TASK_CANCELLED,
            &TaskCancelled { task_id: task.task_id, reason: reason.to_owned() },
            task.correlation_id,
        );
        let messages = message.map(|m| vec![m]).unwrap_or_default();
        if let Err(error) = self.store.set_state(task.task_id, LocalState::Cancelled, &messages) {
            tracing::error!(%error, "cannot cancel the task");
            return;
        }
        self.flush_outbox().await;
    }

    /// Etapa 3: qué se sabe del origen de la tarea (HU-WIN-061).
    ///
    /// Sin firma se devuelve `Claimed`: es la política quien decide si eso basta para ese riesgo.
    /// Con firma, tiene que ser válida y de una clave registrada **para ese principal**; si no, la
    /// tarea se rechaza aquí, porque una firma que no cuadra es peor señal que no traer ninguna.
    fn authenticate(&self, dispatch: &TaskDispatch, principal: &Principal) -> Result<Authenticity, ToolError> {
        let Some(signature) = &dispatch.signature else {
            return Ok(if principal.is_remote() { Authenticity::Claimed } else { Authenticity::Local });
        };
        let policy = self.policy();
        let keys = policy.policy.keys_for(principal);
        if keys.is_empty() {
            return Err(ToolError::new(ErrorCode::SignatureInvalid, "no signing key is registered for that principal"));
        }
        let message = signature::task_message(
            dispatch.task_id,
            &dispatch.task_type,
            &dispatch.payload,
            &dispatch.requested_by,
            dispatch.expires_at,
        );
        match signature::verify(&message, signature, &keys) {
            Ok(key_id) => Ok(Authenticity::Signed(key_id)),
            Err(error) => Err(ToolError::new(ErrorCode::SignatureInvalid, error.to_string())),
        }
    }

    /// Decisión de aprobación llegada por el Cloud, firmada por el dispositivo del owner
    /// (HU-WIN-062). Sin firma válida **no pasa nada**: la aprobación sigue pendiente y el owner
    /// puede decidir por la CLI.
    #[tracing::instrument(skip_all, fields(task_id = %decision.task_id))]
    async fn handle_approval_decision(self: &Arc<Self>, decision: ApprovalDecision) {
        let Ok(Some(approval)) = self.store.approval(decision.task_id) else {
            tracing::warn!("signed decision for a task without a pending approval: ignored");
            return;
        };
        if approval.state != ApprovalState::Pending {
            tracing::warn!(state = ?approval.state, "the approval was already decided: ignored");
            return;
        }
        let granted = match decision.decision.as_str() {
            "GRANTED" => true,
            "DENIED" => false,
            other => {
                tracing::warn!(decision = other, "unknown decision value: ignored");
                return;
            }
        };

        // Frescura: una decisión muy vieja o del futuro no se aplica aunque la firma cuadre.
        let age = (self.health.now() - decision.decided_at).num_seconds().abs();
        if age > DECISION_MAX_AGE_SECONDS {
            tracing::warn!(age_seconds = age, "the signed decision is outside its freshness window: ignored");
            return;
        }

        let policy = self.policy();
        let keys = policy.policy.approver_keys();
        if keys.is_empty() {
            tracing::warn!("no approver key is registered: remote decisions are ignored");
            return;
        }
        // La firma cubre el payload que se aprobó: no vale para el mismo taskId con otro input.
        let message = signature::decision_message(
            decision.task_id,
            &decision.decision,
            &approval.payload_hash,
            &decision.nonce,
            decision.decided_at,
        );
        let key_id = match signature::verify(&message, &decision.signature, &keys) {
            Ok(key_id) => key_id,
            Err(error) => {
                tracing::warn!(%error, "invalid approval signature: ignored");
                return;
            }
        };

        // Anti-replay: el nonce se consume una sola vez, y solo después de validar la firma.
        if decision.nonce.is_empty() || decision.nonce.len() > 128 {
            tracing::warn!("malformed nonce: ignored");
            return;
        }
        match self.store.consume_nonce(&decision.nonce, decision.task_id, self.health.now()) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!("the nonce of this decision was already used: ignored (replay)");
                return;
            }
            Err(error) => {
                tracing::error!(%error, "cannot record the decision nonce; not applying it");
                return;
            }
        }

        tracing::info!(granted, key_id, "applying a signed remote approval decision");
        if let Err(error) = self.decide_approval(decision.task_id, granted, &format!("remote:{key_id}")).await {
            tracing::warn!(%error, "the signed decision could not be applied");
        }
    }

    /// Decisión local de una aprobación pendiente (CLI del owner).
    ///
    /// # Errors
    /// Si la aprobación no existe o ya fue decidida.
    pub async fn decide_approval(self: &Arc<Self>, task_id: Uuid, granted: bool, by: &str) -> anyhow::Result<()> {
        let approval = self.store.approval(task_id)?.ok_or_else(|| anyhow::anyhow!("no pending approval"))?;
        if approval.state != ApprovalState::Pending {
            anyhow::bail!("the approval was already decided ({:?})", approval.state);
        }
        let state = if granted { ApprovalState::Granted } else { ApprovalState::Denied };
        if !self.store.decide_approval(task_id, state, by)? {
            anyhow::bail!("the approval was already decided");
        }
        let task = self.store.get_task(task_id)?.ok_or_else(|| anyhow::anyhow!("unknown task"))?;
        if granted {
            self.store.set_state(task_id, LocalState::Accepted, &[])?;
            self.clone().spawn_execution(task_id);
        } else {
            self.terminate_without_execution(&task, cancel_reasons::APPROVAL_DENIED).await;
        }
        Ok(())
    }

    /// Cancela las aprobaciones vencidas (se llama periódicamente y al arrancar).
    pub async fn expire_approvals(&self) {
        let Ok(expired) = self.store.expired_approvals(self.health.now()) else { return };
        for task_id in expired {
            let _ = self.store.decide_approval(task_id, ApprovalState::Expired, "expiry");
            if let Ok(Some(task)) = self.store.get_task(task_id)
                && !task.state.is_terminal()
            {
                tracing::info!(%task_id, "approval expired");
                self.terminate_without_execution(&task, cancel_reasons::APPROVAL_EXPIRED).await;
            }
        }
    }

    fn spawn_execution(self: Arc<Self>, task_id: Uuid) {
        tokio::spawn(async move {
            if let Err(error) = self.run_task(task_id).await {
                tracing::error!(%task_id, %error, "the task could not be executed");
            }
        });
    }

    /// Etapas 12-16 para una tarea ya aceptada.
    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn run_task(self: &Arc<Self>, task_id: Uuid) -> anyhow::Result<()> {
        let Some(task) = self.store.get_task(task_id)? else { anyhow::bail!("unknown task") };
        if task.state != LocalState::Accepted {
            return Ok(());
        }

        let Some(resolved) = self.registry.resolve_task_type(&task.task_type, &task.payload) else {
            self.fail_task(&task, &ToolError::new(ErrorCode::UnsupportedTaskType, "the tool is no longer available"))
                .await;
            return Ok(());
        };
        let input = match resolved.tool.validate(&resolved.input) {
            Ok(input) => input,
            Err(error) => {
                self.fail_task(&task, &error).await;
                return Ok(());
            }
        };

        // Re-evaluación de política antes de ejecutar (pudo cambiar desde el ack).
        if let Some(principal) = Principal::parse(&task.principal) {
            let risk = resolved.tool.risk(&input);
            let permissions = resolved.tool.required_permissions(&input);
            let authenticity = task.signed_by.clone().map_or_else(
                || {
                    if principal.is_remote() { Authenticity::Claimed } else { Authenticity::Local }
                },
                Authenticity::Signed,
            );
            let decision = jarvis_policy::evaluate(
                &self.policy(),
                &principal,
                resolved.tool.descriptor().name,
                &input,
                risk,
                &permissions,
                &authenticity,
            );
            if decision.kind == DecisionKind::Denied {
                tracing::warn!(reason = decision.reason_code, "denied by policy before executing");
                self.terminate_without_execution(&task, cancel_reasons::LOCAL_POLICY).await;
                return Ok(());
            }
        }

        let decision_label = task.decision.clone().unwrap_or_else(|| "APPROVED".to_owned());
        self.execute_resolved(&task, resolved, input, decision_label, "cloud").await;
        Ok(())
    }

    async fn execute_resolved(
        self: &Arc<Self>,
        task: &TaskRow,
        resolved: Resolved,
        input: Value,
        decision: String,
        invoked_by: &str,
    ) {
        let descriptor = resolved.tool.descriptor().clone();
        let max_duration = task.max_duration_seconds.map(|s| Duration::from_secs(u64::from(s)));
        let timeout = effective_timeout(&descriptor, max_duration, None);
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + timeout;
        self.running.lock().unwrap_or_else(PoisonError::into_inner).insert(task.task_id, cancel.clone());

        let admission = match self.executor.admit(resolved.tool.resources(&input), deadline, &cancel).await {
            Ok(admission) => admission,
            Err(error) => {
                self.running.lock().unwrap_or_else(PoisonError::into_inner).remove(&task.task_id);
                self.fail_task(task, &error).await;
                return;
            }
        };

        let execution_id = Uuid::now_v7();
        let started_at = Utc::now();
        let started =
            self.envelope(mt::TASK_STARTED, &TaskStarted { task_id: task.task_id, started_at }, task.correlation_id);
        // W5/W9: `Running` y la ejecución quedan en disco **antes** de producir ningún efecto.
        if let Err(error) = self.store.start_execution(task.task_id, execution_id, started_at, started.as_slice()) {
            tracing::error!(%error, "cannot record the execution; the task will not run");
            self.running.lock().unwrap_or_else(PoisonError::into_inner).remove(&task.task_id);
            return;
        }
        self.flush_outbox().await;

        let store = self.store.clone();
        let task_id = task.task_id;
        let hooks = RunHooks {
            on_verifying: Some(Arc::new(move || {
                if let Err(error) = store.set_verifying(task_id) {
                    tracing::error!(%error, "cannot record the verifying state");
                }
            })),
        };
        let ctx = ExecContext { execution_id, deadline, cancel };
        let meta = RunMetadata {
            risk_level: task.risk_level.clone().unwrap_or_default(),
            decision,
            invoked_by: invoked_by.to_owned(),
        };
        let result = self.executor.run(admission, resolved.tool, input, ctx, meta, hooks).await;
        self.running.lock().unwrap_or_else(PoisonError::into_inner).remove(&task.task_id);
        self.finish(task, &result).await;
    }

    /// Etapas 15-16: persistir el resultado y publicar el estado terminal.
    async fn finish(&self, task: &TaskRow, result: &ToolResult) {
        let result_json = serde_json::to_string(result).unwrap_or_else(|_| "{}".to_owned());
        let (state, message) = match result.outcome {
            Outcome::Succeeded => (
                LocalState::Succeeded,
                self.envelope(
                    mt::TASK_COMPLETED,
                    &TaskCompleted {
                        task_id: task.task_id,
                        completed_at: result.finished_at,
                        result: serde_json::to_value(result).unwrap_or(Value::Null),
                    },
                    task.correlation_id,
                ),
            ),
            Outcome::Cancelled => (
                LocalState::Cancelled,
                self.envelope(
                    mt::TASK_CANCELLED,
                    &TaskCancelled { task_id: task.task_id, reason: cancel_reasons::USER_REQUEST.to_owned() },
                    task.correlation_id,
                ),
            ),
            Outcome::Failed => {
                let error = result.error.clone();
                (
                    LocalState::Failed,
                    self.envelope(
                        mt::TASK_FAILED,
                        &TaskFailed {
                            task_id: task.task_id,
                            error: ErrorInfo {
                                code: error.as_ref().map_or("EXECUTION_FAILED", |e| e.code.as_str()).to_owned(),
                                message: error.as_ref().map(|e| e.message.clone()).unwrap_or_default(),
                                retryable: error.as_ref().is_some_and(|e| e.retryable),
                            },
                        },
                        task.correlation_id,
                    ),
                )
            }
        };

        tracing::info!(
            tool = %result.tool,
            execution_id = %result.execution_id,
            outcome = ?result.outcome,
            execution = ?result.execution.status,
            verification = ?result.verification.status,
            duration_ms = result.duration_ms,
            "task finished"
        );

        let execution = FinishExecution {
            status: format!("{:?}", result.execution.status).to_uppercase(),
            verification_status: format!("{:?}", result.verification.status).to_uppercase(),
            result_json,
            error_code: result.error.as_ref().map(|e| e.code.as_str().to_owned()),
        };
        let messages = message.map(|m| vec![m]).unwrap_or_default();
        if let Err(error) = self.store.finish_execution(task.task_id, state, &execution, &messages) {
            tracing::error!(%error, "cannot persist the result");
            self.health.raise(codes::DB_ERROR, Severity::Error, error.to_string());
            return;
        }
        self.flush_outbox().await;
    }

    async fn fail_task(&self, task: &TaskRow, error: &ToolError) {
        tracing::warn!(code = %error.code, message = %error.message, "task failed");
        let message = self.envelope(
            mt::TASK_FAILED,
            &TaskFailed {
                task_id: task.task_id,
                error: ErrorInfo {
                    code: error.code.as_str().to_owned(),
                    message: error.message.clone(),
                    retryable: error.retryable,
                },
            },
            task.correlation_id,
        );
        let messages = message.map(|m| vec![m]).unwrap_or_default();
        if let Err(error) = self.store.set_state(task.task_id, LocalState::Failed, &messages) {
            tracing::error!(%error, "cannot persist the failure");
            return;
        }
        self.flush_outbox().await;
    }

    /// Recuperación de arranque (docs/06-state/crash-recovery.md).
    pub async fn recover(self: &Arc<Self>) {
        // Ejecuciones interrumpidas: se reportan como fallidas y **nunca** se repiten (W9).
        match self.store.tasks_in(&[LocalState::Running, LocalState::Verifying]) {
            Ok(interrupted) => {
                for task in interrupted {
                    tracing::warn!(task_id = %task.task_id, "execution interrupted by a restart");
                    self.fail_task(
                        &task,
                        &ToolError::new(
                            ErrorCode::ExecutionInterrupted,
                            "the agent restarted while the task was running",
                        ),
                    )
                    .await;
                }
            }
            Err(error) => tracing::error!(%error, "cannot read interrupted tasks"),
        }

        self.expire_approvals().await;

        // Tareas aceptadas que no llegaron a empezar: vuelven a la cola.
        match self.store.tasks_in(&[LocalState::Accepted]) {
            Ok(queued) => {
                for task in queued {
                    tracing::info!(task_id = %task.task_id, "re-queueing an accepted task");
                    self.clone().spawn_execution(task.task_id);
                }
            }
            Err(error) => tracing::error!(%error, "cannot read queued tasks"),
        }
    }

    /// Identificadores de tareas abiertas para `presence.hello` (W2).
    #[must_use]
    pub fn open_task_ids(&self) -> Vec<Uuid> {
        self.store.open_task_ids(256).unwrap_or_default()
    }

    #[must_use]
    pub fn busy(&self) -> bool {
        self.executor.running() > 0
    }

    /// Invocación local (CLI / futuro Agent Core) sobre el mismo pipeline, sin tarea del Cloud.
    pub async fn invoke_local(
        self: &Arc<Self>,
        tool_name: &str,
        input: Value,
        principal: &Principal,
    ) -> anyhow::Result<ToolResult> {
        let resolved =
            self.registry.resolve_tool(tool_name, input).ok_or_else(|| anyhow::anyhow!("unknown or disabled tool"))?;
        let validated = resolved.tool.validate(&resolved.input).map_err(|e| anyhow::anyhow!("{e}"))?;
        let risk = resolved.tool.risk(&validated);
        let permissions = resolved.tool.required_permissions(&validated);
        let decision = jarvis_policy::evaluate(
            &self.policy(),
            principal,
            resolved.tool.descriptor().name,
            &validated,
            risk,
            &permissions,
            &Authenticity::Local,
        );
        if decision.kind != DecisionKind::Approved {
            anyhow::bail!("{}: {} ({})", decision.kind.as_str(), decision.message, decision.reason_code);
        }
        let descriptor = resolved.tool.descriptor().clone();
        let timeout = effective_timeout(&descriptor, None, None);
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + timeout;
        let admission = self
            .executor
            .admit(resolved.tool.resources(&validated), deadline, &cancel)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let ctx = ExecContext { execution_id: Uuid::now_v7(), deadline, cancel };
        let meta = RunMetadata {
            risk_level: risk.to_string(),
            decision: decision.kind.as_str().to_owned(),
            invoked_by: "local".to_owned(),
        };
        Ok(self.executor.run(admission, resolved.tool, validated, ctx, meta, RunHooks::default()).await)
    }
}
