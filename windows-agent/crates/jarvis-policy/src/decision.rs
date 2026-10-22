//! Evaluación de permisos (docs/04-security/permission-engine.md#algoritmo-de-evaluación).
//!
//! Orden: política válida → R4 remoto → deny → principal → capabilities → umbrales de riesgo.
//! Función pura: mismo input, misma decisión.

use jarvis_tooling::RiskLevel;
use serde_json::Value;

use crate::model::{Policy, PrincipalRule};
use crate::principal::Principal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Approved,
    RequiresApproval,
    Denied,
}

impl DecisionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "APPROVED",
            Self::RequiresApproval => "REQUIRES_APPROVAL",
            Self::Denied => "DENIED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    PolicyInvalid,
    CriticalRemote,
    ExplicitDeny,
    UnknownPrincipal,
    MissingCapability(String),
    RiskTooHigh,
    SignatureRequired,
}

/// Cuánto se sabe del origen de la petición (HU-WIN-061).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authenticity {
    /// Llegó por el pipe local: el propio SO ya autenticó al usuario.
    Local,
    /// Remota sin firma: la identidad es **afirmada** por el Cloud (C10).
    Claimed,
    /// Remota con firma Ed25519 válida de una clave registrada localmente.
    Signed(String),
}

impl Authenticity {
    #[must_use]
    pub const fn is_proven(&self) -> bool {
        matches!(self, Self::Local | Self::Signed(_))
    }
}

impl DenyReason {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PolicyInvalid => "POLICY_INVALID",
            Self::CriticalRemote => "CRITICAL_REMOTE",
            Self::ExplicitDeny => "EXPLICIT_DENY",
            Self::UnknownPrincipal => "UNKNOWN_PRINCIPAL",
            Self::MissingCapability(_) => "MISSING_CAPABILITY",
            Self::RiskTooHigh => "RISK_TOO_HIGH",
            Self::SignatureRequired => "SIGNATURE_REQUIRED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub kind: DecisionKind,
    /// Código del motivo (`AUTO_APPROVED`, `APPROVAL_THRESHOLD`, o el de [`DenyReason`]).
    pub reason_code: &'static str,
    /// Mensaje apto para el Cloud: nombra lo que falta sin revelar la política.
    pub message: String,
    /// Regla aplicada (para auditoría local y `policy check`).
    pub rule: Option<String>,
}

impl Decision {
    fn deny(reason: &DenyReason, message: String, rule: Option<String>) -> Self {
        Self { kind: DecisionKind::Denied, reason_code: reason.code(), message, rule }
    }
}

/// Política efectiva y si es válida (una política inválida conserva la última válida en modo fail-closed).
#[derive(Debug, Clone)]
pub struct PolicyState {
    pub policy: Policy,
    pub valid: bool,
}

/// Evalúa una invocación.
#[must_use]
pub fn evaluate(
    state: &PolicyState,
    principal: &Principal,
    tool: &str,
    input: &Value,
    risk: RiskLevel,
    required_permissions: &[String],
    authenticity: &Authenticity,
) -> Decision {
    let policy = &state.policy;
    let pid = principal.policy_id();

    if !state.valid && risk > RiskLevel::R0 {
        return Decision::deny(
            &DenyReason::PolicyInvalid,
            "local policy is invalid; only R0 tools are allowed".into(),
            None,
        );
    }

    // Firma exigida por política: una identidad solo afirmada no basta a partir del umbral.
    if let Some(threshold) = policy.signatures.require_above
        && risk >= threshold
        && !authenticity.is_proven()
    {
        return Decision::deny(&DenyReason::SignatureRequired, format!("risk {risk} requires a signed request"), None);
    }

    if risk >= RiskLevel::R4 && principal.is_remote() {
        return Decision::deny(&DenyReason::CriticalRemote, format!("risk {risk} is never allowed remotely"), None);
    }

    let app_id = input.get("appId").and_then(Value::as_str);
    if let Some((index, _)) = policy.deny.iter().enumerate().find(|(_, rule)| {
        principal_matches(&rule.principal, principal)
            && rule.tool == tool
            && rule.app_id.as_deref().is_none_or(|a| Some(a) == app_id)
    }) {
        return Decision::deny(
            &DenyReason::ExplicitDeny,
            format!("{tool} is denied for {pid} by local policy"),
            Some(format!("deny[{index}]")),
        );
    }

    let Some(rule) = find_rule(policy, principal) else {
        return Decision::deny(
            &DenyReason::UnknownPrincipal,
            format!("principal {pid} is not allowed by local policy"),
            None,
        );
    };
    let rule_name = format!("principals[{}]", rule.id);

    if let Some(missing) = required_permissions.iter().find(|p| !rule.capabilities.iter().any(|c| c == *p)) {
        return Decision::deny(
            &DenyReason::MissingCapability(missing.clone()),
            format!("principal {pid} lacks {missing}"),
            Some(rule_name),
        );
    }

    let auto_max = rule.auto_approve_max.unwrap_or(policy.risk.auto_approve_max);
    if risk <= auto_max {
        return Decision {
            kind: DecisionKind::Approved,
            reason_code: "AUTO_APPROVED",
            message: String::new(),
            rule: Some(rule_name),
        };
    }
    if risk <= policy.risk.approval_required_max.max(auto_max) {
        return Decision {
            kind: DecisionKind::RequiresApproval,
            reason_code: "APPROVAL_THRESHOLD",
            message: format!("{tool} is {risk} and requires local approval"),
            rule: Some(rule_name),
        };
    }
    Decision::deny(
        &DenyReason::RiskTooHigh,
        format!("{tool} risk {risk} exceeds the local policy limit"),
        Some(rule_name),
    )
}

fn principal_matches(pattern: &str, principal: &Principal) -> bool {
    pattern == "*" || (pattern == "cloud:*" && principal.is_remote()) || pattern == principal.policy_id()
}

/// Regla exacta primero; si no existe y el principal es remoto, `cloud:*`. Las capabilities no se suman.
fn find_rule<'a>(policy: &'a Policy, principal: &Principal) -> Option<&'a PrincipalRule> {
    let id = principal.policy_id();
    policy
        .principals
        .iter()
        .find(|r| r.id == id)
        .or_else(|| principal.is_remote().then(|| policy.principals.iter().find(|r| r.id == "cloud:*")).flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn owner() -> Uuid {
        Uuid::parse_str("0192a6f0-0000-7aaa-8bbb-00000000000a").expect("uuid")
    }
    fn esp32() -> Uuid {
        Uuid::parse_str("0192a6f0-1111-7aaa-8bbb-000000000001").expect("uuid")
    }

    fn state() -> PolicyState {
        let text = format!(
            r#"
version = 1
[[principals]]
id = "cloud:user:{}"
capabilities = ["windows.apps.read", "windows.window.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force"]
[[principals]]
id = "cloud:device:{}"
capabilities = ["windows.apps.read", "windows.apps.open"]
[[principals]]
id = "cloud:*"
capabilities = ["windows.apps.read"]
[[principals]]
id = "local:owner"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force"]
[[deny]]
principal = "cloud:*"
tool = "close_app"
app_id = "vscode"
"#,
            owner(),
            esp32()
        );
        PolicyState { policy: Policy::parse(&text).expect("valid"), valid: true }
    }

    fn perms(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_owned()).collect()
    }

    fn eval(principal: &Principal, tool: &str, input: &Value, risk: RiskLevel, p: &[&str]) -> Decision {
        evaluate(&state(), principal, tool, input, risk, &perms(p), &Authenticity::Claimed)
    }

    #[test]
    fn owner_open_app_is_approved() {
        let d = eval(
            &Principal::CloudUser(owner()),
            "open_app",
            &serde_json::json!({"appId": "chrome"}),
            RiskLevel::R2,
            &["windows.apps.open"],
        );
        assert_eq!(d.kind, DecisionKind::Approved);
    }

    #[test]
    fn missing_capability_is_denied_and_named() {
        let d = eval(
            &Principal::CloudDevice(esp32()),
            "close_app",
            &serde_json::json!({"appId": "chrome"}),
            RiskLevel::R2,
            &["windows.apps.close"],
        );
        assert_eq!(d.kind, DecisionKind::Denied);
        assert_eq!(d.reason_code, "MISSING_CAPABILITY");
        assert!(d.message.contains("windows.apps.close"));
    }

    #[test]
    fn r3_requires_approval_and_r4_remote_is_always_denied() {
        let force = serde_json::json!({"appId": "chrome", "force": true});
        let d = eval(
            &Principal::CloudUser(owner()),
            "close_app",
            &force,
            RiskLevel::R3,
            &["windows.apps.close", "windows.apps.close.force"],
        );
        assert_eq!(d.kind, DecisionKind::RequiresApproval);
        let d = eval(&Principal::CloudUser(owner()), "x", &Value::Null, RiskLevel::R4, &[]);
        assert_eq!(d.reason_code, "CRITICAL_REMOTE");
    }

    #[test]
    fn unknown_remote_principal_falls_back_to_wildcard_without_merging() {
        let stranger = Principal::CloudDevice(Uuid::now_v7());
        let d = eval(&stranger, "is_app_running", &Value::Null, RiskLevel::R0, &["windows.apps.read"]);
        assert_eq!(d.kind, DecisionKind::Approved);
        let d = eval(&stranger, "open_app", &Value::Null, RiskLevel::R2, &["windows.apps.open"]);
        assert_eq!(d.reason_code, "MISSING_CAPABILITY");
        // La regla exacta del ESP32 no hereda nada de cloud:*; y cloud:* no aplica a principals locales.
        let d = eval(&Principal::LocalAgentCore, "is_app_running", &Value::Null, RiskLevel::R0, &["windows.apps.read"]);
        assert_eq!(d.reason_code, "UNKNOWN_PRINCIPAL");
    }

    #[test]
    fn deny_rules_win() {
        let d = eval(
            &Principal::CloudUser(owner()),
            "close_app",
            &serde_json::json!({"appId": "vscode"}),
            RiskLevel::R2,
            &["windows.apps.close"],
        );
        assert_eq!(d.reason_code, "EXPLICIT_DENY");
        let d = eval(
            &Principal::LocalOwner,
            "close_app",
            &serde_json::json!({"appId": "vscode"}),
            RiskLevel::R2,
            &["windows.apps.close"],
        );
        assert_eq!(d.kind, DecisionKind::Approved, "cloud:* deny does not apply locally");
    }

    #[test]
    fn default_policy_does_not_obey_the_cloud() {
        let state = PolicyState { policy: Policy::default(), valid: true };
        let d = evaluate(
            &state,
            &Principal::CloudUser(owner()),
            "is_app_running",
            &Value::Null,
            RiskLevel::R0,
            &perms(&["windows.apps.read"]),
            &Authenticity::Claimed,
        );
        assert_eq!(d.reason_code, "UNKNOWN_PRINCIPAL");
    }

    #[test]
    fn invalid_policy_fails_closed() {
        let mut s = state();
        s.valid = false;
        let d = evaluate(
            &s,
            &Principal::CloudUser(owner()),
            "open_app",
            &Value::Null,
            RiskLevel::R2,
            &perms(&["windows.apps.open"]),
            &Authenticity::Claimed,
        );
        assert_eq!(d.reason_code, "POLICY_INVALID");
        let d = evaluate(
            &s,
            &Principal::CloudUser(owner()),
            "is_app_running",
            &Value::Null,
            RiskLevel::R0,
            &perms(&["windows.apps.read"]),
            &Authenticity::Claimed,
        );
        assert_eq!(d.kind, DecisionKind::Approved);
    }

    #[test]
    fn principal_threshold_can_lower_auto_approval() {
        let mut s = state();
        s.policy.principals[1].auto_approve_max = Some(RiskLevel::R1);
        let d = evaluate(
            &s,
            &Principal::CloudDevice(esp32()),
            "open_app",
            &Value::Null,
            RiskLevel::R2,
            &perms(&["windows.apps.open"]),
            &Authenticity::Claimed,
        );
        assert_eq!(d.kind, DecisionKind::RequiresApproval);
    }
}
