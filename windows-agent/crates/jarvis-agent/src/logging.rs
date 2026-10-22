//! Logging estructurado (docs/07-observability/logging.md).
//!
//! JSON por línea, rotación diaria y contexto por tarea. Reglas de contenido: nunca secretos,
//! contenido de archivos, títulos de ventana ni salida de comandos. Dos barreras lo sostienen:
//! el tipo [`Sensitive`], que no imprime su valor, y la agregación de mensajes repetidos, para que
//! un flood no llene el disco.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// Envoltorio para valores que **no** deben aparecer en logs ni en `Debug`.
///
/// El agente lo usa para todo lo que la política marca como sensible (títulos de ventana, contenido
/// de archivos, salida de comandos): si alguien lo registra por error, solo se ve `***`.
#[derive(Clone, PartialEq, Eq)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Acceso explícito al valor: el nombre deja claro lo que se está haciendo.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T> fmt::Display for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Ventana de agregación de mensajes repetidos.
const AGGREGATION_WINDOW: Duration = Duration::from_secs(60);

/// Capa que descarta repeticiones del mismo mensaje `WARN`/`ERROR` dentro de la ventana y anota
/// cuántas hubo. Un flood de mensajes inválidos no debe llenar el disco (HU-WIN-003).
struct Aggregator {
    seen: Mutex<HashMap<String, (Instant, u32)>>,
}

impl Aggregator {
    fn new() -> Self {
        Self { seen: Mutex::new(HashMap::new()) }
    }

    /// `true` si el evento debe registrarse; el `u32` es cuántos se suprimieron antes.
    fn should_log(&self, key: &str) -> (bool, u32) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        match seen.get_mut(key) {
            Some((first, count)) if now.duration_since(*first) < AGGREGATION_WINDOW => {
                *count += 1;
                (false, *count)
            }
            Some((first, count)) => {
                let suppressed = *count;
                *first = now;
                *count = 0;
                (true, suppressed)
            }
            None => {
                seen.insert(key.to_owned(), (now, 0));
                (true, 0)
            }
        }
    }
}

struct MessageVisitor(Option<String>);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" && self.0.is_none() {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<S: Subscriber> Layer<S> for Aggregator {
    fn event_enabled(&self, event: &Event<'_>, _ctx: Context<'_, S>) -> bool {
        let level = *event.metadata().level();
        if level > tracing::Level::WARN {
            return true; // Solo se agregan WARN y ERROR: el resto pasa tal cual.
        }
        let mut visitor = MessageVisitor(None);
        event.record(&mut visitor);
        let key = format!("{}:{}", event.metadata().target(), visitor.0.unwrap_or_default());
        let (log, suppressed) = self.should_log(&key);
        if log && suppressed > 0 {
            tracing::debug!(repeated = suppressed, "previous message repeated while it was being aggregated");
        }
        log
    }
}

/// Inicializa el logging. Devuelve el guard del escritor asíncrono (debe vivir hasta el apagado).
///
/// # Errors
/// Si no se puede crear el directorio de logs.
pub fn init(directory: &Path, level: &str, to_console: bool) -> anyhow::Result<Option<WorkerGuard>> {
    let filter = EnvFilter::try_from_env("JARVIS_LOG").unwrap_or_else(|_| EnvFilter::new(level));

    if to_console {
        tracing_subscriber::registry()
            .with(filter)
            .with(Aggregator::new())
            .with(tracing_subscriber::fmt::layer().with_target(false))
            .init();
        return Ok(None);
    }

    std::fs::create_dir_all(directory)?;
    let appender = tracing_appender::rolling::daily(directory, "agent.jsonl");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::registry()
        .with(filter)
        .with(Aggregator::new())
        .with(tracing_subscriber::fmt::layer().json().with_current_span(true).with_span_list(false).with_writer(writer))
        .init();
    Ok(Some(guard))
}

/// Borra logs más antiguos que la retención configurada.
pub fn prune(directory: &Path, retention_days: u16) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    let cutoff = std::time::SystemTime::now() - Duration::from_secs(u64::from(retention_days) * 86_400);
    for entry in entries.flatten() {
        let keep = entry.metadata().and_then(|m| m.modified()).is_ok_and(|modified| modified >= cutoff);
        let name = entry.file_name();
        let is_log = name.to_string_lossy().starts_with("agent.jsonl");
        if !keep && is_log {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_never_print_themselves() {
        let title = Sensitive::new("Banco - Saldo 12345".to_owned());
        assert_eq!(format!("{title:?}"), "***");
        assert_eq!(format!("{title}"), "***");
        assert_eq!(title.expose(), "Banco - Saldo 12345");
    }

    #[test]
    fn repeated_warnings_are_aggregated_within_the_window() {
        let aggregator = Aggregator::new();
        assert_eq!(aggregator.should_log("k"), (true, 0), "el primero siempre se registra");
        for _ in 0..10 {
            assert!(!aggregator.should_log("k").0, "las repeticiones se suprimen");
        }
        assert!(aggregator.should_log("other").0, "otro mensaje distinto sí se registra");
    }

    #[test]
    fn old_log_files_are_pruned() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join("agent.jsonl.2020-01-01");
        std::fs::write(&old, "x").expect("write");
        let keep = dir.path().join("agent.jsonl.2026-09-19");
        std::fs::write(&keep, "x").expect("write");

        // Se marca el archivo viejo con una fecha de modificación antigua.
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(60 * 86_400);
        filetime_set(&old, long_ago);

        prune(dir.path(), 14);
        assert!(!old.exists(), "el log antiguo se borra");
        assert!(keep.exists(), "el reciente se conserva");
    }

    /// Ajusta la fecha de modificación sin dependencias externas.
    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        let file = std::fs::OpenOptions::new().write(true).open(path).expect("open");
        file.set_modified(when).expect("set modified");
    }
}
