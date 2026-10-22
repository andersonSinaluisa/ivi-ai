//! Catálogo de errores (docs/03-tools/tool-contract.md#errores).
//!
//! Cada error tiene código estable, categoría y `retryable`. Los mensajes se sanean: nunca stack
//! traces, rutas del perfil del usuario ni más de 500 caracteres.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCategory {
    ValidationError,
    ToolNotFound,
    PermissionDenied,
    ApprovalRequired,
    AppNotFound,
    ExecutionFailed,
    VerificationFailed,
    Timeout,
    Cancelled,
    SystemError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidEnvelope,
    SignatureInvalid,
    InvalidPayload,
    UnsupportedTaskType,
    ExpiredOnArrival,
    NotPermitted,
    ApprovalDenied,
    ApprovalExpired,
    AppNotFound,
    AppElevated,
    AccessDenied,
    SessionUnavailable,
    PathNotAllowed,
    FileNotFound,
    FileTooLarge,
    BinaryFile,
    CommandNotFound,
    CommandFailed,
    ElementNotFound,
    ElementAmbiguous,
    ExecutionFailed,
    VerificationFailed,
    VerificationInconclusive,
    ToolTimeout,
    OutputTooLarge,
    Cancelled,
    ExecutionInterrupted,
    StateReset,
    SystemError,
}

impl ErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidEnvelope => "INVALID_ENVELOPE",
            Self::SignatureInvalid => "SIGNATURE_INVALID",
            Self::InvalidPayload => "INVALID_PAYLOAD",
            Self::UnsupportedTaskType => "UNSUPPORTED_TASK_TYPE",
            Self::ExpiredOnArrival => "EXPIRED_ON_ARRIVAL",
            Self::NotPermitted => "NOT_PERMITTED",
            Self::ApprovalDenied => "APPROVAL_DENIED",
            Self::ApprovalExpired => "APPROVAL_EXPIRED",
            Self::AppNotFound => "APP_NOT_FOUND",
            Self::AppElevated => "APP_ELEVATED",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::SessionUnavailable => "SESSION_UNAVAILABLE",
            Self::PathNotAllowed => "PATH_NOT_ALLOWED",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::FileTooLarge => "FILE_TOO_LARGE",
            Self::BinaryFile => "BINARY_FILE",
            Self::CommandNotFound => "COMMAND_NOT_FOUND",
            Self::CommandFailed => "COMMAND_FAILED",
            Self::ElementNotFound => "ELEMENT_NOT_FOUND",
            Self::ElementAmbiguous => "ELEMENT_AMBIGUOUS",
            Self::ExecutionFailed => "EXECUTION_FAILED",
            Self::VerificationFailed => "VERIFICATION_FAILED",
            Self::VerificationInconclusive => "VERIFICATION_INCONCLUSIVE",
            Self::ToolTimeout => "TOOL_TIMEOUT",
            Self::OutputTooLarge => "OUTPUT_TOO_LARGE",
            Self::Cancelled => "CANCELLED",
            Self::ExecutionInterrupted => "EXECUTION_INTERRUPTED",
            Self::StateReset => "STATE_RESET",
            Self::SystemError => "SYSTEM_ERROR",
        }
    }

    #[must_use]
    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::InvalidEnvelope
            | Self::InvalidPayload
            | Self::ExpiredOnArrival
            | Self::PathNotAllowed
            | Self::CommandNotFound => ErrorCategory::ValidationError,
            Self::UnsupportedTaskType => ErrorCategory::ToolNotFound,
            Self::NotPermitted | Self::SignatureInvalid => ErrorCategory::PermissionDenied,
            Self::ApprovalDenied | Self::ApprovalExpired => ErrorCategory::ApprovalRequired,
            Self::AppNotFound => ErrorCategory::AppNotFound,
            Self::AppElevated
            | Self::AccessDenied
            | Self::SessionUnavailable
            | Self::ExecutionFailed
            | Self::FileNotFound
            | Self::FileTooLarge
            | Self::BinaryFile
            | Self::CommandFailed
            | Self::ElementNotFound
            | Self::ElementAmbiguous
            | Self::OutputTooLarge => ErrorCategory::ExecutionFailed,
            Self::VerificationFailed | Self::VerificationInconclusive => ErrorCategory::VerificationFailed,
            Self::ToolTimeout => ErrorCategory::Timeout,
            Self::Cancelled => ErrorCategory::Cancelled,
            Self::ExecutionInterrupted | Self::StateReset | Self::SystemError => ErrorCategory::SystemError,
        }
    }

    /// `retryable` por defecto del catálogo.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::ApprovalExpired
                | Self::SessionUnavailable
                | Self::VerificationFailed
                | Self::VerificationInconclusive
                | Self::ToolTimeout
                | Self::ElementNotFound
                | Self::ExecutionInterrupted
                | Self::SystemError
        )
    }

    /// Código que viaja en `task.rejected` (solo códigos del catálogo del Cloud, C5).
    #[must_use]
    pub const fn reject_code(self) -> &'static str {
        match self {
            Self::ExpiredOnArrival | Self::StateReset => "EXPIRED_ON_ARRIVAL",
            Self::UnsupportedTaskType => "UNSUPPORTED_TASK_TYPE",
            Self::NotPermitted | Self::SignatureInvalid => "NOT_PERMITTED",
            _ => "INVALID_PAYLOAD",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error estructurado de cualquier etapa (validación, ejecución, verificación…).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

pub const MAX_MESSAGE_CHARS: usize = 500;

impl ToolError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: sanitize_message(&message.into()), retryable: code.retryable() }
    }

    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidPayload, message)
    }
}

/// Recorta a 500 caracteres, sustituye la ruta del perfil por `~` y elimina saltos de línea
/// (evita que un mensaje arrastre trazas de varias líneas).
#[must_use]
pub fn sanitize_message(message: &str) -> String {
    let mut text: String = message.replace(['\r', '\n'], " ");
    if let Ok(profile) = std::env::var("USERPROFILE")
        && profile.len() > 3
    {
        text = replace_case_insensitive(&text, &profile, "~");
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        text = text.chars().take(MAX_MESSAGE_CHARS - 1).collect::<String>() + "…";
    }
    text
}

fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    let lower_hay = haystack.to_lowercase();
    let lower_needle = needle.to_lowercase();
    if lower_hay.len() != haystack.len() || !lower_hay.contains(&lower_needle) {
        return haystack.to_owned();
    }
    let mut out = String::with_capacity(haystack.len());
    let mut rest = 0;
    while let Some(pos) = lower_hay[rest..].find(&lower_needle) {
        out.push_str(&haystack[rest..rest + pos]);
        out.push_str(replacement);
        rest += pos + needle.len();
    }
    out.push_str(&haystack[rest..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_codes_are_cloud_catalog_codes() {
        assert_eq!(ErrorCode::AppNotFound.reject_code(), "INVALID_PAYLOAD");
        assert_eq!(ErrorCode::StateReset.reject_code(), "EXPIRED_ON_ARRIVAL");
        assert_eq!(ErrorCode::SignatureInvalid.reject_code(), "NOT_PERMITTED");
        assert_eq!(ErrorCode::UnsupportedTaskType.reject_code(), "UNSUPPORTED_TASK_TYPE");
    }

    #[test]
    fn categories_and_retryable_follow_the_catalog() {
        assert_eq!(ErrorCode::ToolTimeout.category(), ErrorCategory::Timeout);
        assert!(ErrorCode::VerificationFailed.retryable());
        assert!(!ErrorCode::AppElevated.retryable());
        assert_eq!(
            serde_json::to_string(&ErrorCode::VerificationInconclusive).ok().as_deref(),
            Some("\"VERIFICATION_INCONCLUSIVE\"")
        );
    }

    #[test]
    fn messages_are_sanitized() {
        let long = "x".repeat(900);
        assert_eq!(sanitize_message(&long).chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(sanitize_message("a\nb\r\nc"), "a b  c");
    }
}
