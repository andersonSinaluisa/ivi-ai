//! VERIFY: comprueba el resultado de cada paso. Una ejecución no verificada nunca se guarda como
//! habilidad confiable.

use crate::domain::{RiskClass, ToolOutput, ToolSpec, VerificationStatus};
use crate::tools::ToolError;

pub trait Verifier: Send + Sync {
    fn verify(&self, spec: &ToolSpec, output: &Result<ToolOutput, ToolError>) -> VerificationStatus;
}

#[derive(Default)]
pub struct DefaultVerifier;

impl Verifier for DefaultVerifier {
    fn verify(&self, spec: &ToolSpec, output: &Result<ToolOutput, ToolError>) -> VerificationStatus {
        let Ok(out) = output else { return VerificationStatus::Failed };
        if !out.ok {
            return VerificationStatus::Failed;
        }
        // Si la herramienta trae su propia verificación (windows-agent), manda esa.
        if let Some(v) = out.verification {
            return v;
        }
        // Una lectura que devolvió datos se considera verificada.
        if spec.risk == RiskClass::ReadOnly {
            return if out.data.is_null() { VerificationStatus::Inconclusive } else { VerificationStatus::Passed };
        }
        // Una acción sin evidencia propia no se da por verificada.
        VerificationStatus::Inconclusive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ToolSource;
    use serde_json::json;

    fn spec(risk: RiskClass) -> ToolSpec {
        ToolSpec {
            id: "t".into(),
            name: "t".into(),
            description: "t".into(),
            parameters: json!({"type": "object"}),
            risk,
            permissions: vec![],
            timeout_ms: 1,
            cancellable: false,
            source: ToolSource::Local,
            untrusted_output: false,
        }
    }

    #[test]
    fn verification_rules() {
        let v = DefaultVerifier;
        assert_eq!(v.verify(&spec(RiskClass::ReadOnly), &Ok(ToolOutput::success(json!({"a": 1})))), VerificationStatus::Passed);
        assert_eq!(v.verify(&spec(RiskClass::LowRisk), &Ok(ToolOutput::success(json!({})))), VerificationStatus::Inconclusive);
        assert_eq!(v.verify(&spec(RiskClass::ReadOnly), &Ok(ToolOutput::failure("x"))), VerificationStatus::Failed);
        assert_eq!(v.verify(&spec(RiskClass::ReadOnly), &Err(ToolError::Timeout(1))), VerificationStatus::Failed);
        let mut remote = ToolOutput::success(json!({}));
        remote.verification = Some(VerificationStatus::Passed);
        assert_eq!(v.verify(&spec(RiskClass::LowRisk), &Ok(remote)), VerificationStatus::Passed);
    }
}
