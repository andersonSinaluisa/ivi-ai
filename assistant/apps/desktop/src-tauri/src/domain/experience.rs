use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tool::VerificationStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceStep {
    pub tool: String,
    pub arguments: Value,
    pub ok: bool,
    pub verification: VerificationStatus,
}

/// Una ejecución registrada por el Experience Engine.
///
/// `verified` solo es `true` si **todos** los pasos pasaron la verificación. Una experiencia
/// nunca se convierte sola en habilidad: el usuario la promueve (`promoted`), y solo si está
/// verificada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Experience {
    pub id: String,
    pub goal: String,
    pub context: String,
    pub plan: String,
    pub steps: Vec<ExperienceStep>,
    pub result: String,
    pub evidence: Vec<Value>,
    pub errors: Vec<String>,
    pub procedure: Option<String>,
    pub verified: bool,
    pub promoted: bool,
    pub created_at: DateTime<Utc>,
    /// La tarea terminó y todos sus pasos tuvieron éxito.
    #[serde(default)]
    pub completed: bool,
    /// Veces que se repitió la misma tarea con las mismas herramientas (se agrupan, no se duplican).
    #[serde(default = "one")]
    pub occurrences: u32,
}

fn one() -> u32 {
    1
}

impl Experience {
    /// Firma para detectar duplicados: objetivo normalizado + secuencia de herramientas.
    #[must_use]
    pub fn signature(&self) -> String {
        use sha2::{Digest, Sha256};
        let goal: String = self
            .goal
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let tools: Vec<&str> = self.steps.iter().map(|s| s.tool.as_str()).collect();
        let digest = Sha256::digest(format!("{goal}|{}", tools.join(",")).as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Procedimiento reutilizable derivado de los pasos (solo si se verificó).
    #[must_use]
    pub fn derive_procedure(steps: &[ExperienceStep]) -> Option<String> {
        if steps.is_empty() || !steps.iter().all(|s| s.ok && s.verification == VerificationStatus::Passed) {
            return None;
        }
        let lines: Vec<String> =
            steps.iter().enumerate().map(|(i, s)| format!("{}. {} {}", i + 1, s.tool, s.arguments)).collect();
        Some(lines.join("\n"))
    }
}
