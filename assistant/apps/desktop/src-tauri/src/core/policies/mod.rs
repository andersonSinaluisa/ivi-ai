//! PolicyEngine: decide si una llamada se ejecuta, se confirma con el usuario o se deniega.
//!
//! Reglas, en orden:
//! 1. Una herramienta denegada (lista `deny` o modo `deny`) nunca se ejecuta.
//! 2. DESTRUCTIVE siempre requiere confirmación, aunque se configure `allow`.
//! 3. Si en esta ejecución ya entró contenido no confiable (archivos, webs, ventanas), todo lo que
//!    no sea READ_ONLY requiere confirmación: defensa contra prompt injection.
//! 4. Modo por herramienta (`allow` / `confirm`) configurado por el usuario.
//! 5. Clases de riesgo auto-aprobadas (por defecto READ_ONLY y LOW_RISK).
//!
//! Estas reglas viven en código y no dependen de lo que diga el modelo ni la personalidad.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::{RiskClass, ToolSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    #[default]
    Default,
    Allow,
    Confirm,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyConfig {
    pub auto_approve: BTreeSet<RiskClass>,
    pub deny: BTreeSet<String>,
    #[serde(default)]
    pub tool_modes: BTreeMap<String, ToolMode>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            auto_approve: [RiskClass::ReadOnly, RiskClass::LowRisk].into_iter().collect(),
            deny: BTreeSet::new(),
            tool_modes: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Confirm { reason: String },
    Deny { reason: String },
}

impl Decision {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Confirm { .. } => "confirm",
            Self::Deny { .. } => "deny",
        }
    }
}

/// Estado de seguridad de una ejecución.
#[derive(Debug, Clone, Default)]
pub struct RunSecurity {
    /// Ya entró al contexto salida de una herramienta no confiable.
    pub tainted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    #[must_use]
    pub fn new(mut config: PolicyConfig) -> Self {
        // DESTRUCTIVE nunca se auto-aprueba, aunque el archivo lo pida.
        config.auto_approve.remove(&RiskClass::Destructive);
        Self { config }
    }

    #[must_use]
    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    pub fn set_tool_mode(&mut self, tool: &str, mode: ToolMode) {
        if mode == ToolMode::Default {
            self.config.tool_modes.remove(tool);
        } else {
            self.config.tool_modes.insert(tool.to_owned(), mode);
        }
    }

    pub fn set_auto_approve(&mut self, classes: BTreeSet<RiskClass>) {
        self.config.auto_approve = classes;
        self.config.auto_approve.remove(&RiskClass::Destructive);
    }

    #[must_use]
    pub fn mode(&self, tool: &str) -> ToolMode {
        self.config.tool_modes.get(tool).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn is_denied(&self, spec: &ToolSpec) -> bool {
        self.config.deny.contains(&spec.name) || self.mode(&spec.name) == ToolMode::Deny
    }

    #[must_use]
    pub fn decide(&self, spec: &ToolSpec, run: &RunSecurity) -> Decision {
        if self.is_denied(spec) {
            return Decision::Deny { reason: format!("la herramienta '{}' está bloqueada por la configuración", spec.name) };
        }
        if spec.risk == RiskClass::Destructive {
            return Decision::Confirm { reason: "acción destructiva: siempre requiere confirmación".into() };
        }
        if run.tainted && spec.risk != RiskClass::ReadOnly {
            return Decision::Confirm {
                reason: "esta tarea ya leyó contenido externo; por seguridad confirma la acción".into(),
            };
        }
        match self.mode(&spec.name) {
            ToolMode::Allow => return Decision::Allow,
            ToolMode::Confirm => return Decision::Confirm { reason: "configurada para pedir confirmación".into() },
            ToolMode::Default | ToolMode::Deny => {}
        }
        if self.config.auto_approve.contains(&spec.risk) {
            Decision::Allow
        } else {
            Decision::Confirm { reason: format!("acción de riesgo {}", spec.risk.label_es()) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ToolSource;
    use serde_json::json;

    fn spec(name: &str, risk: RiskClass) -> ToolSpec {
        ToolSpec {
            id: name.into(),
            name: name.into(),
            description: "x".into(),
            parameters: json!({"type": "object"}),
            risk,
            permissions: vec![],
            timeout_ms: 1000,
            cancellable: false,
            source: ToolSource::Local,
            untrusted_output: false,
        }
    }

    #[test]
    fn default_matrix() {
        let p = PolicyEngine::new(PolicyConfig::default());
        let clean = RunSecurity::default();
        assert_eq!(p.decide(&spec("a", RiskClass::ReadOnly), &clean), Decision::Allow);
        assert_eq!(p.decide(&spec("b", RiskClass::LowRisk), &clean), Decision::Allow);
        assert!(matches!(p.decide(&spec("c", RiskClass::Sensitive), &clean), Decision::Confirm { .. }));
        assert!(matches!(p.decide(&spec("d", RiskClass::Destructive), &clean), Decision::Confirm { .. }));
    }

    #[test]
    fn destructive_always_confirms_even_if_allowed() {
        let mut cfg = PolicyConfig::default();
        cfg.auto_approve.insert(RiskClass::Destructive);
        let mut p = PolicyEngine::new(cfg);
        p.set_tool_mode("wipe", ToolMode::Allow);
        assert!(matches!(p.decide(&spec("wipe", RiskClass::Destructive), &RunSecurity::default()), Decision::Confirm { .. }));
    }

    #[test]
    fn deny_wins_over_everything() {
        let mut cfg = PolicyConfig::default();
        cfg.deny.insert("get_current_time".into());
        let p = PolicyEngine::new(cfg);
        let s = spec("get_current_time", RiskClass::ReadOnly);
        assert!(p.is_denied(&s));
        assert!(matches!(p.decide(&s, &RunSecurity::default()), Decision::Deny { .. }));
    }

    #[test]
    fn tainted_runs_escalate_non_read_only_actions() {
        let p = PolicyEngine::new(PolicyConfig::default());
        let tainted = RunSecurity { tainted: true };
        assert_eq!(p.decide(&spec("a", RiskClass::ReadOnly), &tainted), Decision::Allow);
        assert!(matches!(p.decide(&spec("open_app", RiskClass::LowRisk), &tainted), Decision::Confirm { .. }));
    }

    #[test]
    fn per_tool_modes_apply() {
        let mut p = PolicyEngine::new(PolicyConfig::default());
        p.set_tool_mode("open_app", ToolMode::Confirm);
        p.set_tool_mode("close_app", ToolMode::Allow);
        let clean = RunSecurity::default();
        assert!(matches!(p.decide(&spec("open_app", RiskClass::LowRisk), &clean), Decision::Confirm { .. }));
        assert_eq!(p.decide(&spec("close_app", RiskClass::Sensitive), &clean), Decision::Allow);
        p.set_tool_mode("close_app", ToolMode::Default);
        assert!(matches!(p.decide(&spec("close_app", RiskClass::Sensitive), &clean), Decision::Confirm { .. }));
    }
}
