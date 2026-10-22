//! Configuración no sensible (`config.toml`). Los secretos **nunca** viven aquí
//! (docs/06-state/local-state.md#configuración-local, ADR-009).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Nombres de clave que delatan un secreto pegado en la configuración.
const FORBIDDEN_KEYS: &[&str] = &["password", "secret", "token", "credential", "apikey", "api_key"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {reason}")]
    Io { path: String, reason: String },
    #[error("invalid config.toml: {0}")]
    Invalid(String),
    #[error(
        "config.toml contains the key '{0}': secrets are never stored in configuration files. \
         Use `jarvis-agent provision` (Windows Credential Manager)."
    )]
    SecretInConfig(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct DeviceConfig {
    /// Identificador del device en el Cloud (no es un secreto).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CloudConfig {
    /// `mqtts://host:8883` (TLS) o `mqtt://127.0.0.1:1883` (solo loopback, desarrollo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker_url: Option<String>,
    /// CA propia en PEM para entornos de desarrollo con certificado no público.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MqttConfig {
    #[serde(default = "keepalive")]
    pub keepalive_seconds: u16,
    #[serde(default = "inflight")]
    pub max_inflight: u16,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self { keepalive_seconds: keepalive(), max_inflight: inflight() }
    }
}

const fn keepalive() -> u16 {
    60
}
const fn inflight() -> u16 {
    16
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ToolsConfig {
    #[serde(default = "default_tools")]
    pub enabled: Vec<String>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self { enabled: default_tools() }
    }
}

fn default_tools() -> Vec<String> {
    ["is_app_running", "list_running_apps", "get_active_window", "open_app", "close_app"].map(String::from).to_vec()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExecutorConfig {
    #[serde(default = "max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "queue_capacity")]
    pub queue_capacity: usize,
    /// Tiempo sin entrada del usuario para considerar la sesión `Idle`.
    #[serde(default = "idle_seconds")]
    pub idle_threshold_seconds: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: max_concurrent(),
            queue_capacity: queue_capacity(),
            idle_threshold_seconds: idle_seconds(),
        }
    }
}

const fn max_concurrent() -> usize {
    4
}
const fn queue_capacity() -> usize {
    32
}
const fn idle_seconds() -> u64 {
    600
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct LoggingConfig {
    #[serde(default = "level")]
    pub level: String,
    #[serde(default = "retention")]
    pub retention_days: u16,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: level(), retention_days: retention() }
    }
}

fn level() -> String {
    "info".to_owned()
}
const fn retention() -> u16 {
    14
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Config {
    #[serde(default)]
    pub device: DeviceConfig,
    #[serde(default)]
    pub cloud: CloudConfig,
    #[serde(default)]
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub executor: ExecutorConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub tool_api: ToolApiConfig,
}

/// Local Tool API para el futuro Agent Core (EPIC-16). Apagada por defecto: mientras no haya un
/// Agent Core, abrir el canal solo añadiría superficie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ToolApiConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Presupuesto de llamadas por minuto del principal `local:agent-core`.
    #[serde(default = "default_calls_per_minute")]
    pub max_calls_per_minute: usize,
}

impl Default for ToolApiConfig {
    fn default() -> Self {
        Self { enabled: false, max_calls_per_minute: default_calls_per_minute() }
    }
}

const fn default_calls_per_minute() -> usize {
    60
}

/// Origen de las actualizaciones. Deshabilitado por defecto: mientras no lo esté, el agente solo se
/// actualiza cuando el owner ejecuta `jarvis-agent update` con un manifiesto explícito.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct UpdateConfig {
    #[serde(default)]
    pub enabled: bool,
    /// URL `https` del manifiesto firmado. El Cloud no interviene: esta ruta la fija el owner.
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default = "stable")]
    pub channel: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self { enabled: false, manifest_url: None, channel: stable() }
    }
}

fn stable() -> String {
    "stable".to_owned()
}

impl Config {
    /// Carga y valida. Archivo ausente = valores por defecto (estado `NOT_PROVISIONED`).
    ///
    /// # Errors
    /// [`ConfigError`] con el motivo; el agente no arranca con configuración inválida (fail fast).
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Io { path: path.display().to_string(), reason: e.to_string() }),
        };
        Self::parse(&text)
    }

    /// # Errors
    /// [`ConfigError::Invalid`] o [`ConfigError::SecretInConfig`].
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        for line in text.lines() {
            let key = line.split('=').next().unwrap_or_default().trim().trim_matches('"').to_ascii_lowercase();
            if FORBIDDEN_KEYS.contains(&key.as_str()) {
                return Err(ConfigError::SecretInConfig(key));
            }
        }
        let config: Self = toml::from_str(text).map_err(|e| ConfigError::Invalid(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(url) = &self.cloud.broker_url {
            BrokerUrl::parse(url).map_err(ConfigError::Invalid)?;
        }
        if self.executor.max_concurrent == 0 || self.executor.max_concurrent > 32 {
            return Err(ConfigError::Invalid("executor.max_concurrent must be in 1..=32".into()));
        }
        if self.tools.enabled.is_empty() {
            return Err(ConfigError::Invalid("tools.enabled must not be empty".into()));
        }
        if let Some(url) = &self.update.manifest_url
            && !url.starts_with("https://")
        {
            return Err(ConfigError::Invalid("update.manifest_url must be an https URL".into()));
        }
        if self.update.enabled && self.update.manifest_url.is_none() {
            return Err(ConfigError::Invalid("update.enabled requires update.manifest_url".into()));
        }
        if !matches!(self.update.channel.as_str(), "stable" | "beta") {
            return Err(ConfigError::Invalid("update.channel must be 'stable' or 'beta'".into()));
        }
        if !(1..=600).contains(&self.tool_api.max_calls_per_minute) {
            return Err(ConfigError::Invalid("tool_api.max_calls_per_minute must be in 1..=600".into()));
        }
        Ok(())
    }

    /// Serializa la configuración (sin secretos: no los contiene por diseño).
    ///
    /// # Errors
    /// Si la serialización TOML falla.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::Invalid(e.to_string()))
    }

    #[must_use]
    pub fn idle_threshold(&self) -> Duration {
        Duration::from_secs(self.executor.idle_threshold_seconds)
    }
}

/// URL del broker ya validada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerUrl {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

impl BrokerUrl {
    /// Acepta `mqtts://host[:puerto]` y, **solo para loopback**, `mqtt://host[:puerto]`.
    ///
    /// Desviación documentada respecto a docs/02-cloud-integration/mqtt-client.md: el entorno de
    /// desarrollo del Cloud expone MQTT sin TLS en 127.0.0.1. Fuera de loopback, TLS es obligatorio.
    ///
    /// # Errors
    /// Mensaje explicando por qué la URL no es aceptable.
    pub fn parse(url: &str) -> Result<Self, String> {
        let (scheme, rest) = url.split_once("://").ok_or_else(|| format!("invalid broker url '{url}'"))?;
        let tls = match scheme {
            "mqtts" | "ssl" | "tls" => true,
            "mqtt" | "tcp" => false,
            other => return Err(format!("unsupported scheme '{other}' (use mqtts:// or mqtt://)")),
        };
        let rest = rest.trim_end_matches('/');
        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().map_err(|_| format!("invalid port in '{url}'"))?),
            None => (rest, if tls { 8883 } else { 1883 }),
        };
        if host.is_empty() {
            return Err("broker host is empty".into());
        }
        if !tls && !is_loopback(host) {
            return Err(format!(
                "plaintext MQTT is only allowed for loopback hosts (got '{host}'); use mqtts:// for remote brokers"
            ));
        }
        Ok(Self { host: host.to_owned(), port, tls })
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let config = Config::load(Path::new("does-not-exist.toml")).expect("defaults");
        assert_eq!(config.tools.enabled.len(), 5);
        assert!(config.device.device_id.is_none());
        assert_eq!(config.mqtt.keepalive_seconds, 60);
    }

    #[test]
    fn refuses_to_start_with_a_secret_in_the_config() {
        let text = "[cloud]\nbroker_url = \"mqtts://example.com:8883\"\npassword = \"hunter2\"\n";
        assert!(matches!(Config::parse(text), Err(ConfigError::SecretInConfig(k)) if k == "password"));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        assert!(matches!(Config::parse("[cloud]\nnope = 1"), Err(ConfigError::Invalid(_))));
        assert!(matches!(Config::parse("[executor]\nmax_concurrent = 0"), Err(ConfigError::Invalid(_))));
        assert!(matches!(Config::parse("[tools]\nenabled = []"), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn broker_urls_require_tls_outside_loopback() {
        assert_eq!(
            BrokerUrl::parse("mqtts://jarvis.example.com"),
            Ok(BrokerUrl { host: "jarvis.example.com".into(), port: 8883, tls: true })
        );
        assert_eq!(
            BrokerUrl::parse("mqtt://127.0.0.1:1883"),
            Ok(BrokerUrl { host: "127.0.0.1".into(), port: 1883, tls: false })
        );
        assert!(BrokerUrl::parse("mqtt://jarvis.example.com:1883").is_err(), "sin TLS solo loopback");
        assert!(BrokerUrl::parse("http://jarvis.example.com").is_err());
        assert!(BrokerUrl::parse("jarvis.example.com").is_err());
    }

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.device.device_id = Some(Uuid::now_v7());
        config.cloud.broker_url = Some("mqtts://example.com:8883".into());
        let text = config.to_toml().expect("serialize");
        assert_eq!(Config::parse(&text).expect("parse"), config);
        assert!(!text.to_lowercase().contains("password"));
    }
}
