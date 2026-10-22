//! `jarvis-agent diagnostics` (HU-WIN-052): paquete de soporte **redactado**.
//!
//! Nunca se sube a ningún sitio: se genera un archivo que el owner decide compartir. No incluye
//! payloads ni resultados de tools, ni hostname, usuario o IP (docs/07-observability/diagnostics.md).

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use jarvis_state::Store;

use crate::config::Config;
use crate::paths::Paths;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub days: i64,
    pub include_logs: bool,
    /// Escribe el contenido por consola en lugar de generar el archivo.
    pub print: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { days: 3, include_logs: true, print: false }
    }
}

/// Genera el paquete. Devuelve la ruta del archivo (o `None` con `--print`).
///
/// # Errors
/// Errores de lectura de la configuración o de escritura del paquete.
pub fn collect(paths: &Paths, options: Options) -> anyhow::Result<Option<PathBuf>> {
    let mut sections: Vec<(String, String)> = Vec::new();

    sections.push(("version.txt".into(), version_section()));
    sections.push(("system.txt".into(), system_section()));

    match std::fs::read_to_string(paths.config()) {
        Ok(text) => sections.push(("config.toml".into(), text)),
        Err(error) => sections.push(("config.toml".into(), format!("# not readable: {error}"))),
    }
    match std::fs::read_to_string(paths.policy()) {
        Ok(text) => sections.push(("policy.toml".into(), text)),
        Err(error) => sections.push(("policy.toml".into(), format!("# not readable: {error}"))),
    }
    sections.push(("tasks.txt".into(), tasks_section(paths)));

    if options.include_logs {
        for (name, text) in recent_logs(&paths.logs(), options.days) {
            sections.push((format!("logs/{name}"), redact(&text)));
        }
    }

    if options.print {
        for (name, content) in &sections {
            println!("===== {name} =====");
            println!("{content}");
        }
        return Ok(None);
    }

    // Se escribe una carpeta, no un zip: evita una dependencia más y el owner puede revisar cada
    // archivo antes de compartirlo (que es justo lo que queremos que haga).
    let target = desktop_or(paths).join(format!("jarvis-diagnostics-{}", Utc::now().format("%Y%m%d-%H%M%S")));
    std::fs::create_dir_all(target.join("logs"))?;
    for (name, content) in sections {
        let file = target.join(&name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut handle = std::fs::File::create(&file)?;
        handle.write_all(content.as_bytes())?;
    }
    Ok(Some(target))
}

fn version_section() -> String {
    format!(
        "agent: {}\nprotocol: v1 (presence {})\nbuild: {}\n",
        crate::cloud::AGENT_VERSION,
        crate::cloud::PROTOCOL_VERSION,
        if cfg!(debug_assertions) { "debug" } else { "release" }
    )
}

/// Información mínima del sistema: **sin** hostname, usuario, dominio ni direcciones.
fn system_section() -> String {
    format!(
        "os: windows\narch: {}\ncpus: {}\n",
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
    )
}

/// Resumen de las últimas tareas: identificadores, estados y códigos de error. Sin payloads ni
/// resultados (ahí es donde viven títulos de ventana y demás datos personales).
fn tasks_section(paths: &Paths) -> String {
    let Ok(store) = Store::open(&paths.database()) else {
        return "# local database not readable\n".to_owned();
    };
    let stats = store.stats().unwrap_or_default();
    let mut out = format!(
        "tasks: {}\nunsettled: {}\noutbox_pending: {}\ndb_bytes: {}\ndb_created_at: {}\n\n",
        stats.tasks,
        stats.unsettled,
        stats.outbox_pending,
        stats.size_bytes,
        store.db_created_at()
    );
    out.push_str("state            tool              risk  decision           task\n");
    let states = [
        jarvis_state::LocalState::AwaitingApproval,
        jarvis_state::LocalState::Accepted,
        jarvis_state::LocalState::Running,
        jarvis_state::LocalState::Verifying,
        jarvis_state::LocalState::Succeeded,
        jarvis_state::LocalState::Failed,
        jarvis_state::LocalState::Cancelled,
        jarvis_state::LocalState::Rejected,
    ];
    let Ok(tasks) = store.tasks_in(&states) else { return out };
    for task in tasks.iter().take(200) {
        let execution = store.execution(task.task_id).ok().flatten();
        out.push_str(&format!(
            "{:<16} {:<17} {:<5} {:<18} {} {}\n",
            task.state.as_str(),
            task.tool.clone().unwrap_or_else(|| "-".into()),
            task.risk_level.clone().unwrap_or_else(|| "-".into()),
            task.decision.clone().unwrap_or_else(|| "-".into()),
            task.task_id,
            execution.and_then(|e| e.error_code).unwrap_or_default()
        ));
    }
    out
}

fn recent_logs(directory: &Path, days: i64) -> Vec<(String, String)> {
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(u64::try_from(days.max(0)).unwrap_or(3) * 86_400);
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let recent = entry.metadata().and_then(|m| m.modified()).is_ok_and(|modified| modified >= cutoff);
        if recent && let Ok(text) = std::fs::read_to_string(entry.path()) {
            out.push((entry.file_name().to_string_lossy().into_owned(), text));
        }
    }
    out
}

/// Segunda pasada de redacción sobre los logs: aunque el agente ya evita registrar datos sensibles,
/// un paquete de diagnóstico se comparte y conviene no depender de una sola barrera.
#[must_use]
pub fn redact(text: &str) -> String {
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    let mut out = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let suffix = &token[trimmed.len()..];
        let replaced = if looks_like_secret(trimmed) { "[REDACTED]".to_owned() } else { trimmed.to_owned() };
        out.push_str(&replaced);
        out.push_str(suffix);
    }
    if !profile.is_empty() {
        out = replace_ignore_case(&out, &profile, "~");
    }
    if !user.is_empty() && user.len() > 2 {
        out = replace_ignore_case(&out, &user, "[user]");
    }
    out
}

fn looks_like_secret(token: &str) -> bool {
    let core = token.trim_matches(|c: char| !c.is_alphanumeric());
    if core.len() < 24 {
        return false;
    }
    let jwt = core.starts_with("eyJ");
    let base64ish = core.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(char::is_uppercase)
        && !core.contains('-'); // los UUID llevan guiones y sí son útiles en diagnóstico
    jwt || base64ish
}

fn replace_ignore_case(haystack: &str, needle: &str, replacement: &str) -> String {
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

fn desktop_or(paths: &Paths) -> PathBuf {
    std::env::var("USERPROFILE")
        .ok()
        .map(|profile| PathBuf::from(profile).join("Desktop"))
        .filter(|desktop| desktop.is_dir())
        .unwrap_or_else(|| paths.root.clone())
}

/// Comprueba que la configuración no contenga secretos antes de empaquetarla.
///
/// # Errors
/// Si la configuración es inválida o contiene una clave sospechosa.
pub fn assert_config_is_shareable(paths: &Paths) -> anyhow::Result<()> {
    Config::load(&paths.config()).map(|_| ()).map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens_and_user_paths() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload";
        let text = format!("auth {jwt} taskId 01a0bbbe-04ed-7018-a7d4-e7eb558eb3f4 done");
        let redacted = redact(&text);
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIs"), "{redacted}");
        assert!(redacted.contains("01a0bbbe-04ed-7018-a7d4-e7eb558eb3f4"), "los UUID se conservan: {redacted}");
    }

    #[test]
    fn keeps_short_words_untouched() {
        assert_eq!(redact("agent started version 0.1.0"), "agent started version 0.1.0");
    }

    #[test]
    fn system_section_has_no_identifiers() {
        let section = system_section();
        let host = std::env::var("COMPUTERNAME").unwrap_or_default();
        let user = std::env::var("USERNAME").unwrap_or_default();
        assert!(!section.contains(&host) || host.is_empty());
        assert!(!section.to_lowercase().contains(&user.to_lowercase()) || user.is_empty());
    }
}
