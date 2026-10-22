//! Health snapshot local (docs/07-observability/health.md). Sin endpoint HTTP: el agente no abre
//! puertos; se consulta por el named pipe.

use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentState {
    Starting,
    Running,
    Stopping,
    SafeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub code: String,
    pub severity: Severity,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub agent: AgentSection,
    pub mqtt: MqttSection,
    pub cloud: CloudSection,
    pub db: DbSection,
    pub tools: ToolsSection,
    pub policy: PolicySection,
    pub session: SessionSection,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSection {
    pub state: AgentState,
    pub version: String,
    pub uptime_seconds: i64,
    pub elevated: bool,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MqttSection {
    pub connected: bool,
    pub since: Option<DateTime<Utc>>,
    pub reconnects: u64,
    pub last_error: Option<String>,
    pub broker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSection {
    pub reachable: bool,
    pub last_hello_ack: Option<DateTime<Utc>>,
    pub clock_offset_ms: i64,
    pub auth_rejected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbSection {
    pub healthy: bool,
    pub size_bytes: u64,
    pub outbox_pending: u64,
    pub unsettled: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsSection {
    pub healthy: bool,
    pub enabled: usize,
    pub running: usize,
    pub awaiting_approval: usize,
    pub stuck: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicySection {
    pub valid: bool,
    pub loaded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSection {
    pub state: String,
}

/// Estado mutable compartido del health.
#[derive(Debug)]
pub struct Health {
    inner: Mutex<Inner>,
    started_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Inner {
    agent_state: AgentState,
    device_id: Option<String>,
    elevated: bool,
    connected: bool,
    connected_since: Option<DateTime<Utc>>,
    reconnects: u64,
    last_error: Option<String>,
    broker: Option<String>,
    hello_ack: Option<DateTime<Utc>>,
    clock_offset_ms: i64,
    auth_rejected: bool,
    db_healthy: bool,
    policy_valid: bool,
    policy_loaded_at: Option<DateTime<Utc>>,
    session_state: String,
    conditions: Vec<Condition>,
}

impl Default for Health {
    fn default() -> Self {
        Self::new()
    }
}

impl Health {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Utc::now(),
            inner: Mutex::new(Inner {
                agent_state: AgentState::Starting,
                device_id: None,
                elevated: false,
                connected: false,
                connected_since: None,
                reconnects: 0,
                last_error: None,
                broker: None,
                hello_ack: None,
                clock_offset_ms: 0,
                auth_rejected: false,
                db_healthy: true,
                policy_valid: true,
                policy_loaded_at: None,
                session_state: "Unknown".to_owned(),
                conditions: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_agent_state(&self, state: AgentState) {
        self.lock().agent_state = state;
    }

    pub fn set_identity(&self, device_id: Option<String>, elevated: bool, broker: Option<String>) {
        let mut inner = self.lock();
        inner.device_id = device_id;
        inner.elevated = elevated;
        inner.broker = broker;
    }

    pub fn set_connected(&self, connected: bool, error: Option<String>) {
        let mut inner = self.lock();
        if connected && !inner.connected {
            inner.connected_since = Some(Utc::now());
        }
        if !connected {
            inner.connected_since = None;
            inner.reconnects += 1;
            inner.hello_ack = None;
        }
        inner.connected = connected;
        inner.last_error = error;
    }

    /// `true` si ahora mismo hay conexión con el Cloud (lo usa la ventana de confirmación de una
    /// actualización: una versión que no llega a conectar no se da por buena).
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.lock().connected
    }

    pub fn set_hello_ack(&self, at: DateTime<Utc>, offset_ms: i64) {
        let mut inner = self.lock();
        inner.hello_ack = Some(at);
        inner.clock_offset_ms = offset_ms;
        inner.auth_rejected = false;
    }

    pub fn set_auth_rejected(&self, rejected: bool) {
        self.lock().auth_rejected = rejected;
    }

    pub fn set_policy(&self, valid: bool) {
        let mut inner = self.lock();
        inner.policy_valid = valid;
        inner.policy_loaded_at = Some(Utc::now());
    }

    pub fn set_session_state(&self, state: &str) {
        self.lock().session_state = state.to_owned();
    }

    /// Añade (o refresca) una condición activa.
    pub fn raise(&self, code: &str, severity: Severity, detail: impl Into<String>) {
        let mut inner = self.lock();
        let detail = detail.into();
        if let Some(existing) = inner.conditions.iter_mut().find(|c| c.code == code) {
            existing.severity = severity;
            existing.detail = detail;
        } else {
            inner.conditions.push(Condition { code: code.to_owned(), severity, detail });
        }
    }

    pub fn clear(&self, code: &str) {
        self.lock().conditions.retain(|c| c.code != code);
    }

    #[must_use]
    pub fn clock_offset_ms(&self) -> i64 {
        self.lock().clock_offset_ms
    }

    /// Hora corregida con el desfase informado por el Cloud (`hello.ack`).
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::milliseconds(self.clock_offset_ms())
    }

    #[must_use]
    pub fn snapshot(&self, db: DbSection, tools: ToolsSection, version: &str) -> Snapshot {
        let inner = self.lock();
        Snapshot {
            agent: AgentSection {
                state: inner.agent_state,
                version: version.to_owned(),
                uptime_seconds: (Utc::now() - self.started_at).num_seconds(),
                elevated: inner.elevated,
                device_id: inner.device_id.clone(),
            },
            mqtt: MqttSection {
                connected: inner.connected,
                since: inner.connected_since,
                reconnects: inner.reconnects,
                last_error: inner.last_error.clone(),
                broker: inner.broker.clone(),
            },
            cloud: CloudSection {
                reachable: inner.hello_ack.is_some() && inner.connected,
                last_hello_ack: inner.hello_ack,
                clock_offset_ms: inner.clock_offset_ms,
                auth_rejected: inner.auth_rejected,
            },
            db: DbSection { healthy: inner.db_healthy, ..db },
            tools,
            policy: PolicySection { valid: inner.policy_valid, loaded_at: inner.policy_loaded_at },
            session: SessionSection { state: inner.session_state.clone() },
            conditions: inner.conditions.clone(),
        }
    }
}

/// Códigos de condición usados por el agente.
pub mod codes {
    pub const NOT_PROVISIONED: &str = "NOT_PROVISIONED";
    pub const AUTH_REJECTED: &str = "AUTH_REJECTED";
    pub const CLOUD_UNREACHABLE: &str = "CLOUD_UNREACHABLE";
    pub const CLOCK_SKEW: &str = "CLOCK_SKEW";
    pub const POLICY_INVALID: &str = "POLICY_INVALID";
    pub const STATE_RESET: &str = "STATE_RESET";
    pub const OUTBOX_STUCK: &str = "OUTBOX_STUCK";
    pub const DB_ERROR: &str = "DB_ERROR";
    pub const RUNNING_ELEVATED: &str = "RUNNING_ELEVATED";
    pub const TOOL_STUCK: &str = "TOOL_STUCK";
    pub const SAFE_MODE: &str = "SAFE_MODE";
    /// La versión nueva no arrancó bien y se volvió a la anterior (HU-WIN-063).
    pub const UPDATE_ROLLED_BACK: &str = "UPDATE_ROLLED_BACK";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_are_raised_and_cleared_once() {
        let health = Health::new();
        health.raise(codes::CLOCK_SKEW, Severity::Warn, "offset 6 min");
        health.raise(codes::CLOCK_SKEW, Severity::Warn, "offset 7 min");
        let snapshot = health.snapshot(
            DbSection { healthy: true, size_bytes: 0, outbox_pending: 0, unsettled: 0 },
            ToolsSection { healthy: true, enabled: 5, running: 0, awaiting_approval: 0, stuck: 0 },
            "0.1.0",
        );
        assert_eq!(snapshot.conditions.len(), 1);
        assert_eq!(snapshot.conditions[0].detail, "offset 7 min");
        health.clear(codes::CLOCK_SKEW);
        let snapshot = health.snapshot(
            DbSection { healthy: true, size_bytes: 0, outbox_pending: 0, unsettled: 0 },
            ToolsSection { healthy: true, enabled: 5, running: 0, awaiting_approval: 0, stuck: 0 },
            "0.1.0",
        );
        assert!(snapshot.conditions.is_empty());
    }

    #[test]
    fn corrected_clock_uses_the_reported_offset() {
        let health = Health::new();
        health.set_hello_ack(Utc::now(), 5_000);
        let delta = (health.now() - Utc::now()).num_milliseconds();
        assert!((4_000..=6_000).contains(&delta), "delta {delta}");
    }
}
