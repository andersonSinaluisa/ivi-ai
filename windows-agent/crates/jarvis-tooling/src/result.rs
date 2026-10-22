//! `ToolResult` estándar (`jarvis.tool-result/1`, docs/03-tools/tool-contract.md#toolresult).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ErrorCategory, ErrorCode, ToolError};

pub const SCHEMA: &str = "jarvis.tool-result/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationStatus {
    Passed,
    Failed,
    Inconclusive,
    NotApplicable,
    /// No se verificó porque la ejecución no tuvo éxito.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<serde_json::Value>,
}

impl Check {
    #[must_use]
    pub fn new(name: &str, passed: bool, observed: Option<serde_json::Value>) -> Self {
        Self { name: name.to_owned(), passed, observed }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSection {
    pub status: ExecutionStatus,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSection {
    pub status: VerificationStatus,
    pub method: String,
    pub duration_ms: u64,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorSection {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub category: ErrorCategory,
}

impl From<&ToolError> for ErrorSection {
    fn from(e: &ToolError) -> Self {
        Self { code: e.code, message: e.message.clone(), retryable: e.retryable, category: e.code.category() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub risk_level: String,
    pub decision: String,
    /// `cloud` | `local`.
    pub invoked_by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub schema: String,
    pub tool: String,
    pub tool_version: String,
    pub execution_id: Uuid,
    pub outcome: Outcome,
    #[serde(with = "ts")]
    pub started_at: DateTime<Utc>,
    #[serde(with = "ts")]
    pub finished_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub execution: ExecutionSection,
    pub verification: VerificationSection,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    pub truncated: bool,
    #[serde(default)]
    pub error: Option<ErrorSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

impl ToolResult {
    /// Regla del principio de verificación: éxito solo si la ejecución tuvo éxito **y** la
    /// verificación pasó (o no aplica por ser una tool de observación).
    #[must_use]
    pub const fn derive_outcome(execution: ExecutionStatus, verification: VerificationStatus) -> Outcome {
        match (execution, verification) {
            (ExecutionStatus::Succeeded, VerificationStatus::Passed | VerificationStatus::NotApplicable) => {
                Outcome::Succeeded
            }
            (ExecutionStatus::Cancelled, _) => Outcome::Cancelled,
            _ => Outcome::Failed,
        }
    }
}

mod ts {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.to_rfc3339_opts(SecondsFormat::Millis, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&text).map(|t| t.with_timezone(&Utc)).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_requires_passed_or_not_applicable_verification() {
        use ExecutionStatus as E;
        use VerificationStatus as V;
        assert_eq!(ToolResult::derive_outcome(E::Succeeded, V::Passed), Outcome::Succeeded);
        assert_eq!(ToolResult::derive_outcome(E::Succeeded, V::NotApplicable), Outcome::Succeeded);
        assert_eq!(ToolResult::derive_outcome(E::Succeeded, V::Failed), Outcome::Failed);
        assert_eq!(ToolResult::derive_outcome(E::Succeeded, V::Inconclusive), Outcome::Failed);
        assert_eq!(ToolResult::derive_outcome(E::Failed, V::Skipped), Outcome::Failed);
        assert_eq!(ToolResult::derive_outcome(E::TimedOut, V::Skipped), Outcome::Failed);
        assert_eq!(ToolResult::derive_outcome(E::Cancelled, V::Skipped), Outcome::Cancelled);
    }
}
