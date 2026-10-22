//! Modelo y carga de `policy.toml` (docs/04-security/permission-engine.md#política-policytoml).

use std::collections::BTreeMap;
use std::path::Path;

use jarvis_tooling::RiskLevel;
use serde::Deserialize;

use crate::apps::{AppCatalog, AppEntry, is_valid_app_id};
use crate::files::{CommandEntry, FilesystemPolicy, validate_command};
use crate::principal::Principal;

/// Capabilities conocidas. No hay comodines (`windows.*`) ni `shell.execute`.
pub const KNOWN_CAPABILITIES: &[&str] = &[
    "windows.apps.read",
    "windows.apps.open",
    "windows.apps.close",
    "windows.apps.close.force",
    "windows.window.read",
    "windows.window.title",
    "windows.window.focus",
    "windows.notify",
    "filesystem.list",
    "filesystem.read",
    "filesystem.write",
    "commands.run",
    "windows.ui.read",
    "windows.ui.act",
];

/// Capabilities de procesos del MVP hasta R2 (lo que `provision` concede al owner).
pub const OWNER_DEFAULT_CAPABILITIES: &[&str] =
    &["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.window.read"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("cannot read policy file: {0}")]
    Io(String),
    #[error("invalid policy.toml: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskThresholds {
    #[serde(default = "r2", deserialize_with = "de_risk")]
    pub auto_approve_max: RiskLevel,
    #[serde(default = "r3", deserialize_with = "de_risk")]
    pub approval_required_max: RiskLevel,
    #[serde(default = "ttl")]
    pub approval_ttl_seconds: u64,
}

impl Default for RiskThresholds {
    fn default() -> Self {
        Self { auto_approve_max: RiskLevel::R2, approval_required_max: RiskLevel::R3, approval_ttl_seconds: 300 }
    }
}

const fn r2() -> RiskLevel {
    RiskLevel::R2
}
const fn r3() -> RiskLevel {
    RiskLevel::R3
}
const fn ttl() -> u64 {
    300
}

/// Convierte las claves ya validadas al cargar. Una clave que aquí no compile se descarta: no
/// puede ocurrir porque `parse` las valida, y descartarla es más seguro que hacer `unwrap`.
fn to_keys(entries: &[KeyEntry]) -> Vec<jarvis_protocol::PublicKey> {
    entries
        .iter()
        .filter_map(|entry| jarvis_protocol::PublicKey::from_base64(entry.id.clone(), &entry.key).ok())
        .collect()
}

/// Una clave mal formada invalida la política entera: es mejor no arrancar que aceptar firmas
/// que nunca se podrán verificar.
fn check_key(key: &KeyEntry, owner: &str) -> Result<(), PolicyError> {
    if key.id.is_empty() || key.id.len() > 64 {
        return Err(PolicyError::Invalid(format!("{owner}: key id must be 1..=64 characters")));
    }
    jarvis_protocol::PublicKey::from_base64(key.id.clone(), &key.key)
        .map(|_| ())
        .map_err(|error| PolicyError::Invalid(format!("{owner}: key '{}' is invalid ({error})", key.id)))
}

fn de_risk<'de, D: serde::Deserializer<'de>>(d: D) -> Result<RiskLevel, D::Error> {
    String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
}

fn de_risk_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<RiskLevel>, D::Error> {
    Option::<String>::deserialize(d)?.map(|s| s.parse().map_err(serde::de::Error::custom)).transpose()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRule {
    /// `cloud:user:<id>`, `cloud:device:<id>`, `cloud:*`, `local:owner`, `local:agent-core`.
    pub id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, deserialize_with = "de_risk_opt")]
    pub auto_approve_max: Option<RiskLevel>,
    /// Claves Ed25519 con las que este principal puede firmar tareas (HU-WIN-061).
    #[serde(default)]
    pub public_keys: Vec<KeyEntry>,
}

/// Clave pública registrada localmente: el Cloud no puede añadirlas ni cambiarlas.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEntry {
    /// `keyId` que viajará en la firma.
    pub id: String,
    /// Clave Ed25519 (32 bytes en base64).
    pub key: String,
}

/// Exigencia de firma y claves de aprobadores remotos.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignaturePolicy {
    /// A partir de qué riesgo se exige firma válida en la tarea. Ausente = no se exige.
    ///
    /// El Cloud manda `signature: null` en v1 ([C10]), así que el valor por defecto mantiene el
    /// agente operativo; el owner lo sube cuando su Cloud firme.
    ///
    /// [C10]: ../../00-overview/cloud-compatibility.md
    #[serde(default, deserialize_with = "de_risk_opt")]
    pub require_above: Option<RiskLevel>,
    /// Claves de los dispositivos que pueden decidir aprobaciones en remoto (HU-WIN-062).
    #[serde(default)]
    pub approver_keys: Vec<KeyEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenyRule {
    /// Patrón de principal: id exacto, `cloud:*` o `*`.
    pub principal: String,
    pub tool: String,
    /// Restringe la regla a un `appId` concreto.
    #[serde(default)]
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleMode {
    Full,
    AppOnly,
    #[default]
    RedactPatterns,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    #[serde(default)]
    pub title_mode: TitleMode,
    /// Procesos cuyo título nunca se devuelve.
    #[serde(default = "never_title")]
    pub never_title: Vec<String>,
}

impl Default for Privacy {
    fn default() -> Self {
        Self { title_mode: TitleMode::RedactPatterns, never_title: never_title() }
    }
}

fn never_title() -> Vec<String> {
    ["KeePassXC.exe", "KeePass.exe", "1Password.exe", "Bitwarden.exe", "mstsc.exe"].map(String::from).to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    version: u32,
    #[serde(default)]
    risk: RiskThresholds,
    #[serde(default)]
    principals: Vec<PrincipalRule>,
    #[serde(default)]
    deny: Vec<DenyRule>,
    #[serde(default)]
    apps: BTreeMap<String, AppEntry>,
    #[serde(default)]
    privacy: Privacy,
    #[serde(default)]
    filesystem: FilesystemPolicy,
    #[serde(default)]
    commands: BTreeMap<String, CommandEntry>,
    #[serde(default)]
    signatures: SignaturePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub risk: RiskThresholds,
    pub principals: Vec<PrincipalRule>,
    pub deny: Vec<DenyRule>,
    pub apps: AppCatalog,
    pub privacy: Privacy,
    /// Vacío por defecto: sin raíces, las tools de filesystem rechazan todo.
    pub filesystem: FilesystemPolicy,
    /// Vacío por defecto: sin comandos declarados, `run_command` no tiene nada que ejecutar.
    pub commands: BTreeMap<String, CommandEntry>,
    /// Firmas: qué se exige y quién puede aprobar en remoto.
    pub signatures: SignaturePolicy,
}

impl Default for Policy {
    /// Sin archivo: solo `local:owner`; **ningún principal remoto** (el agente no obedece al Cloud
    /// hasta que el owner lo decide).
    fn default() -> Self {
        Self {
            risk: RiskThresholds::default(),
            principals: vec![PrincipalRule {
                id: "local:owner".into(),
                capabilities: OWNER_DEFAULT_CAPABILITIES.iter().map(|s| (*s).to_owned()).collect(),
                auto_approve_max: None,
                public_keys: Vec::new(),
            }],
            deny: Vec::new(),
            apps: AppCatalog::defaults(),
            privacy: Privacy::default(),
            filesystem: FilesystemPolicy::default(),
            commands: BTreeMap::new(),
            signatures: SignaturePolicy::default(),
        }
    }
}

impl Policy {
    /// Carga y valida un archivo. Archivo inexistente = política por defecto.
    ///
    /// # Errors
    /// [`PolicyError`] con la línea del problema.
    pub fn load(path: &Path) -> Result<Self, PolicyError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(PolicyError::Io(e.to_string())),
        }
    }

    /// # Errors
    /// [`PolicyError::Invalid`] si el TOML o las reglas no son válidos.
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let file: PolicyFile = toml::from_str(text).map_err(|e| PolicyError::Invalid(e.to_string()))?;
        if file.version != 1 {
            return Err(PolicyError::Invalid(format!("unsupported version {}", file.version)));
        }
        validate_risk(&file.risk)?;
        for rule in &file.principals {
            if rule.id != "cloud:*" && Principal::parse(&rule.id).is_none() {
                return Err(PolicyError::Invalid(format!("unknown principal '{}'", rule.id)));
            }
            for cap in &rule.capabilities {
                if !KNOWN_CAPABILITIES.contains(&cap.as_str()) && !cap.starts_with("commands.run:") {
                    return Err(PolicyError::Invalid(format!("unknown capability '{cap}' for '{}'", rule.id)));
                }
            }
            if rule.auto_approve_max.is_some_and(|r| r >= RiskLevel::R4) {
                return Err(PolicyError::Invalid(format!("'{}': R4 can never be auto-approved", rule.id)));
            }
        }
        for rule in &file.deny {
            let pattern_ok =
                rule.principal == "*" || rule.principal == "cloud:*" || Principal::parse(&rule.principal).is_some();
            if !pattern_ok {
                return Err(PolicyError::Invalid(format!("deny rule: unknown principal pattern '{}'", rule.principal)));
            }
        }
        for (id, entry) in &file.apps {
            if !is_valid_app_id(id) {
                return Err(PolicyError::Invalid(format!("app id '{id}' must match [a-z0-9_-]{{1,64}}")));
            }
            if entry.process_names.is_empty() {
                return Err(PolicyError::Invalid(format!("app '{id}' needs at least one process name")));
            }
        }
        for (id, entry) in &file.commands {
            validate_command(id, entry).map_err(PolicyError::Invalid)?;
        }
        for rule in &file.principals {
            for key in &rule.public_keys {
                check_key(key, &rule.id)?;
            }
        }
        for key in &file.signatures.approver_keys {
            check_key(key, "signatures.approver_keys")?;
        }
        if file.signatures.require_above == Some(RiskLevel::R0) {
            return Err(PolicyError::Invalid("signatures.require_above must be R1 or higher".into()));
        }
        let mut apps = AppCatalog::defaults();
        apps.extend(file.apps);
        Ok(Self {
            risk: file.risk,
            principals: file.principals,
            deny: file.deny,
            apps,
            privacy: file.privacy,
            filesystem: file.filesystem,
            commands: file.commands,
            signatures: file.signatures,
        })
    }

    /// Claves con las que ese principal puede firmar tareas.
    ///
    /// Se busca la regla exacta y, si no existe y el principal es remoto, `cloud:*`. Las claves
    /// **no se suman** entre reglas, igual que las capabilities: una regla concreta manda entera.
    #[must_use]
    pub fn keys_for(&self, principal: &Principal) -> Vec<jarvis_protocol::PublicKey> {
        let id = principal.policy_id();
        let rule =
            self.principals.iter().find(|rule| rule.id == id).or_else(|| {
                principal.is_remote().then(|| self.principals.iter().find(|r| r.id == "cloud:*")).flatten()
            });
        rule.map(|rule| to_keys(&rule.public_keys)).unwrap_or_default()
    }

    /// Claves de los dispositivos que pueden decidir aprobaciones en remoto.
    #[must_use]
    pub fn approver_keys(&self) -> Vec<jarvis_protocol::PublicKey> {
        to_keys(&self.signatures.approver_keys)
    }

    /// Texto de una política inicial para el owner (lo escribe `provision`).
    #[must_use]
    pub fn owner_template(owner_user_id: Option<uuid::Uuid>) -> String {
        let caps = OWNER_DEFAULT_CAPABILITIES.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
        let mut text = String::from(
            "# Jarvis Windows Agent - politica local (docs/04-security/permission-engine.md)\n\
             # El Cloud no puede leer ni modificar este archivo.\nversion = 1\n\n\
             [risk]\nauto_approve_max = \"R2\"\napproval_required_max = \"R3\"\napproval_ttl_seconds = 300\n\n",
        );
        text.push_str(&format!("[[principals]]\nid = \"local:owner\"\ncapabilities = [{caps}]\n\n"));
        if let Some(owner) = owner_user_id {
            text.push_str(&format!(
                "# Owner via Jarvis Cloud (requestedBy User)\n[[principals]]\nid = \"cloud:user:{}\"\ncapabilities = [{caps}]\n",
                owner.hyphenated()
            ));
        }
        text
    }
}

fn validate_risk(risk: &RiskThresholds) -> Result<(), PolicyError> {
    if risk.auto_approve_max >= RiskLevel::R4 || risk.approval_required_max >= RiskLevel::R4 {
        return Err(PolicyError::Invalid("R4 can never be approved remotely".into()));
    }
    if risk.approval_required_max < risk.auto_approve_max {
        return Err(PolicyError::Invalid("approval_required_max must be >= auto_approve_max".into()));
    }
    if risk.approval_ttl_seconds == 0 || risk.approval_ttl_seconds > 86_400 {
        return Err(PolicyError::Invalid("approval_ttl_seconds must be in 1..=86400".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_has_no_remote_principals() {
        let policy = Policy::default();
        assert!(policy.principals.iter().all(|p| p.id.starts_with("local:")));
        assert!(policy.apps.get("chrome").is_some());
    }

    #[test]
    fn parses_a_full_policy_and_merges_apps() {
        let text = r#"
version = 1
[risk]
auto_approve_max = "R1"
[[principals]]
id = "cloud:device:0192a6f0-1111-7aaa-8bbb-000000000001"
capabilities = ["windows.apps.read", "windows.apps.open"]
[[principals]]
id = "cloud:*"
capabilities = ["windows.apps.read"]
[[deny]]
principal = "cloud:*"
tool = "close_app"
app_id = "vscode"
[apps.vscode]
display_name = "Visual Studio Code"
aliases = ["code"]
launch = { kind = "app_paths", name = "Code.exe" }
process_names = ["Code.exe"]
"#;
        let policy = Policy::parse(text).expect("valid");
        assert_eq!(policy.risk.auto_approve_max, RiskLevel::R1);
        assert_eq!(policy.principals.len(), 2);
        assert!(policy.apps.get("vscode").is_some());
        assert!(policy.apps.get("chrome").is_some(), "defaults are kept");
    }

    #[test]
    fn rejects_invalid_policies() {
        let cases = [
            ("version = 2", "unsupported version"),
            ("version = 1\nunknown = 1", "unknown field"),
            ("version = 1\n[[principals]]\nid = \"cloud:admin\"", "unknown principal"),
            (
                "version = 1\n[[principals]]\nid = \"local:owner\"\ncapabilities = [\"shell.execute\"]",
                "unknown capability",
            ),
            ("version = 1\n[risk]\nauto_approve_max = \"R4\"", "R4"),
            ("version = 1\n[risk]\nauto_approve_max = \"R3\"\napproval_required_max = \"R2\"", ">="),
            (
                "version = 1\n[apps.\"Bad Id\"]\ndisplay_name=\"x\"\nlaunch={kind=\"path\",path=\"c:/x.exe\"}\nprocess_names=[\"x.exe\"]",
                "app id",
            ),
        ];
        for (text, expected) in cases {
            let err = Policy::parse(text).expect_err(text).to_string();
            assert!(err.contains(expected), "{text}: {err}");
        }
    }

    #[test]
    fn owner_template_is_a_valid_policy() {
        let owner = uuid::Uuid::now_v7();
        let policy = Policy::parse(&Policy::owner_template(Some(owner))).expect("valid template");
        assert!(policy.principals.iter().any(|p| p.id == format!("cloud:user:{owner}")));
    }
}
