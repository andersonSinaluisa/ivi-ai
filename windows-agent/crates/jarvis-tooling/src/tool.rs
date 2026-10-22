//! Trait `Tool` (ADR-003). Una tool no conoce el transporte: recibe un input JSON ya validado y
//! un contexto con deadline y token de cancelación.

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::descriptor::ToolDescriptor;
use crate::error::ToolError;
use crate::locks::ResourceLock;
use crate::result::{Check, VerificationStatus};
use crate::risk::RiskLevel;

/// Contexto de una ejecución.
#[derive(Debug, Clone)]
pub struct ExecContext {
    pub execution_id: Uuid,
    pub deadline: tokio::time::Instant,
    pub cancel: CancellationToken,
}

impl ExecContext {
    /// Duerme `d` salvo que llegue la cancelación. Devuelve `Err(Cancelled)` si se canceló.
    ///
    /// # Errors
    /// [`crate::ErrorCode::Cancelled`] si el token se disparó.
    pub async fn sleep(&self, d: std::time::Duration) -> Result<(), ToolError> {
        tokio::select! {
            () = self.cancel.cancelled() => Err(ToolError::new(crate::ErrorCode::Cancelled, "cancelled")),
            () = tokio::time::sleep(d) => Ok(()),
        }
    }
}

/// Resultado de `execute`: el `output` para el `ToolResult` y la evidencia que usará `verify`.
#[derive(Debug, Clone, Default)]
pub struct ExecutionOutcome {
    pub output: Value,
    /// Detalles de cómo se ejecutó (sin datos sensibles), p. ej. `{ "method": "wm_close" }`.
    pub details: Option<Value>,
    /// Evidencia interna para la verificación (no se publica), p. ej. PID lanzado e instante.
    pub evidence: Value,
}

/// Informe de verificación.
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub status: VerificationStatus,
    pub checks: Vec<Check>,
    /// Mensaje para `task.failed` cuando la verificación no pasa (sin títulos ni rutas).
    pub message: Option<String>,
    /// La verificación puede completar el output con lo observado (p. ej. PID real de la app).
    pub output_patch: Option<Value>,
}

impl VerificationReport {
    #[must_use]
    pub const fn not_applicable() -> Self {
        Self { status: VerificationStatus::NotApplicable, checks: Vec::new(), message: None, output_patch: None }
    }

    #[must_use]
    pub fn passed(checks: Vec<Check>) -> Self {
        Self { status: VerificationStatus::Passed, checks, message: None, output_patch: None }
    }

    #[must_use]
    pub fn failed(checks: Vec<Check>, message: impl Into<String>) -> Self {
        Self { status: VerificationStatus::Failed, checks, message: Some(message.into()), output_patch: None }
    }

    #[must_use]
    pub fn inconclusive(checks: Vec<Check>, message: impl Into<String>) -> Self {
        Self { status: VerificationStatus::Inconclusive, checks, message: Some(message.into()), output_patch: None }
    }

    #[must_use]
    pub fn with_output_patch(mut self, patch: Value) -> Self {
        self.output_patch = Some(patch);
        self
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> &ToolDescriptor;

    /// Validación pura (sin efectos). Devuelve el input **normalizado** que usará el resto del pipeline.
    ///
    /// # Errors
    /// `INVALID_PAYLOAD` / `APP_NOT_FOUND` con un mensaje legible.
    fn validate(&self, input: &Value) -> Result<Value, ToolError>;

    /// Riesgo efectivo: nunca menor que el riesgo base.
    fn risk(&self, _input: &Value) -> RiskLevel {
        self.descriptor().risk_level
    }

    /// Permisos requeridos para este input (todos deben estar concedidos).
    fn required_permissions(&self, _input: &Value) -> Vec<String> {
        self.descriptor().required_permissions.iter().map(|p| (*p).to_owned()).collect()
    }

    /// Recursos que hay que bloquear para este input.
    fn resources(&self, _input: &Value) -> Vec<ResourceLock> {
        Vec::new()
    }

    /// Produce el efecto. No decide permisos y no reintenta por su cuenta.
    ///
    /// # Errors
    /// Error estructurado del catálogo.
    async fn execute(&self, input: &Value, ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError>;

    /// Solo observa: comprueba la condición de éxito. Nunca repite la acción.
    async fn verify(&self, _input: &Value, _outcome: &ExecutionOutcome, _ctx: &ExecContext) -> VerificationReport {
        VerificationReport::not_applicable()
    }
}
