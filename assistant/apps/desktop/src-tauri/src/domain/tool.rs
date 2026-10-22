use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Clasificación de riesgo de una herramienta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskClass {
    ReadOnly,
    LowRisk,
    Sensitive,
    Destructive,
}

impl RiskClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "READ_ONLY",
            Self::LowRisk => "LOW_RISK",
            Self::Sensitive => "SENSITIVE",
            Self::Destructive => "DESTRUCTIVE",
        }
    }

    /// Nombre para mostrar al usuario.
    #[must_use]
    pub const fn label_es(self) -> &'static str {
        match self {
            Self::ReadOnly => "solo lectura",
            Self::LowRisk => "bajo",
            Self::Sensitive => "sensible",
            Self::Destructive => "destructiva",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "READ_ONLY" => Some(Self::ReadOnly),
            "LOW_RISK" => Some(Self::LowRisk),
            "SENSITIVE" => Some(Self::Sensitive),
            "DESTRUCTIVE" => Some(Self::Destructive),
            _ => None,
        }
    }

    /// Traducción de los niveles R0…R4 del windows-agent.
    ///
    /// R0 observar → READ_ONLY · R1 lectura sensible y R2 acción reversible → LOW_RISK ·
    /// R3 acción con impacto → SENSITIVE · R4 crítica → DESTRUCTIVE.
    /// El windows-agent aplica además su propia política (R3 exige su aprobación local).
    #[must_use]
    pub fn from_agent_level(level: &str) -> Self {
        match level {
            "R0" => Self::ReadOnly,
            "R1" | "R2" => Self::LowRisk,
            "R3" => Self::Sensitive,
            // Un nivel desconocido se trata como el peor caso.
            _ => Self::Destructive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Local,
    WindowsAgent,
}

/// Descriptor de una herramienta. Es lo que el LLM ve (nombre, descripción, parámetros) y lo que
/// la política usa (riesgo, permisos, timeout).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    /// JSON Schema de los argumentos.
    pub parameters: Value,
    pub risk: RiskClass,
    pub permissions: Vec<String>,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub source: ToolSource,
    /// La salida puede contener texto de terceros (archivos, webs): se trata como no confiable
    /// para la defensa contra prompt injection.
    pub untrusted_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    Inconclusive,
    NotApplicable,
}

/// Resultado estructurado de una herramienta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Verificación que ya trae la herramienta (p. ej. el `ToolResult` del windows-agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationStatus>,
}

impl ToolOutput {
    #[must_use]
    pub fn success(data: Value) -> Self {
        Self { ok: true, data, error: None, verification: None }
    }

    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self { ok: false, data: Value::Null, error: Some(message.into()), verification: None }
    }
}

#[cfg(test)]
mod tests {
    use super::RiskClass;

    #[test]
    fn agent_levels_map_to_risk_classes() {
        assert_eq!(RiskClass::from_agent_level("R0"), RiskClass::ReadOnly);
        assert_eq!(RiskClass::from_agent_level("R2"), RiskClass::LowRisk);
        assert_eq!(RiskClass::from_agent_level("R3"), RiskClass::Sensitive);
        assert_eq!(RiskClass::from_agent_level("R4"), RiskClass::Destructive);
        assert_eq!(RiskClass::from_agent_level("??"), RiskClass::Destructive, "desconocido = peor caso");
    }

    #[test]
    fn risk_classes_round_trip() {
        for r in [RiskClass::ReadOnly, RiskClass::LowRisk, RiskClass::Sensitive, RiskClass::Destructive] {
            assert_eq!(RiskClass::parse(r.as_str()), Some(r));
            let json = serde_json::to_string(&r).expect("json");
            assert_eq!(json, format!("\"{}\"", r.as_str()));
        }
    }
}
