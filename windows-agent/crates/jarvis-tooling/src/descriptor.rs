//! Descriptor estático de una tool (docs/03-tools/tool-contract.md#descriptor-estático).

use std::time::Duration;

use serde::Serialize;

use crate::risk::RiskLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationMode {
    /// No cancelable: se informa `cancellation not supported by tool` y se reporta el estado real.
    None,
    /// La tool revisa el token entre pasos.
    Cooperative,
    /// La tool termina su árbol de procesos (Job Object).
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    /// Tools con efectos: la condición observable es obligatoria.
    Required,
    /// Solo tools de observación pura: la observación es el resultado.
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    None,
    /// El output puede contener datos personales: se redacta tras *settled* y no se loguea.
    Output,
    InputAndOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeouts {
    #[serde(rename = "defaultTimeoutMs", serialize_with = "ser_ms")]
    pub default: Duration,
    #[serde(rename = "maxTimeoutMs", serialize_with = "ser_ms")]
    pub max: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSpec {
    pub mode: VerificationMode,
    /// Nombre estable del método (`process_and_window`, `process_absent`…).
    pub method: &'static str,
    #[serde(rename = "timeoutMs", serialize_with = "ser_ms")]
    pub timeout: Duration,
    #[serde(rename = "pollIntervalMs", serialize_with = "ser_ms")]
    pub poll_interval: Duration,
}

impl VerificationSpec {
    #[must_use]
    pub const fn not_applicable() -> Self {
        Self {
            mode: VerificationMode::NotApplicable,
            method: "none",
            timeout: Duration::ZERO,
            poll_interval: Duration::ZERO,
        }
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // firma impuesta por serde
fn ser_ms<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Límite duro del sistema: ninguna tool puede declarar más de 24 h.
pub const MAX_TOOL_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub risk_level: RiskLevel,
    /// Permisos base (la tool puede añadir más según el input).
    pub required_permissions: Vec<&'static str>,
    /// JSON Schema (draft 2020-12) del input, expuesto para clientes y el futuro Agent Core.
    /// La validación efectiva la hace `Tool::validate` con tipos estrictos.
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub timeouts: Timeouts,
    pub cancellation: CancellationMode,
    pub verification: VerificationSpec,
    pub sensitivity: Sensitivity,
    pub max_output_bytes: usize,
    pub requires_interactive_session: bool,
    pub idempotent_effect: bool,
}

/// Tope del `output` para que el `ToolResult` quepa en los 32 KiB de `result` del Cloud.
pub const MAX_OUTPUT_BYTES: usize = 24 * 1024;

impl ToolDescriptor {
    /// Valida las reglas estáticas del contrato. Un descriptor inválido impide arrancar el agente.
    ///
    /// # Errors
    /// Mensaje con la regla incumplida.
    pub fn check(&self) -> Result<(), String> {
        let name_ok = !self.name.is_empty()
            && self.name.len() <= 64
            && self.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !name_ok {
            return Err(format!("tool name '{}' must be snake_case", self.name));
        }
        if self.timeouts.max.is_zero() || self.timeouts.max > MAX_TOOL_TIMEOUT {
            return Err(format!("{}: maxTimeout must be in (0, 24h]", self.name));
        }
        if self.timeouts.default.is_zero() || self.timeouts.default > self.timeouts.max {
            return Err(format!("{}: defaultTimeout must be in (0, maxTimeout]", self.name));
        }
        if self.risk_level >= RiskLevel::R2 && self.verification.mode != VerificationMode::Required {
            return Err(format!("{}: tools with effects (R2+) must declare a required verification", self.name));
        }
        if self.verification.mode == VerificationMode::Required
            && (self.verification.timeout.is_zero() || self.verification.poll_interval.is_zero())
        {
            return Err(format!("{}: verification timeout and poll interval must be > 0", self.name));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(format!("{}: maxOutputBytes must be in (0, {MAX_OUTPUT_BYTES}]", self.name));
        }
        if self.required_permissions.is_empty() {
            return Err(format!("{}: at least one required permission", self.name));
        }
        Ok(())
    }
}
