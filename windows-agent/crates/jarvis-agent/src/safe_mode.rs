//! Modo seguro ante *crash loop* (HU-WIN-018, docs/06-state/crash-recovery.md#crash-loop).
//!
//! Si el agente cae varias veces seguidas, una tarea "envenenada" podría recibirse en bucle. Tras
//! 3 arranques sin apagado ordenado en 10 minutos, el agente sigue publicando lo pendiente pero
//! **rechaza tareas nuevas** hasta que el owner lo reconozca o pase una hora sin caídas.

use chrono::{DateTime, Duration, Utc};
use jarvis_state::Store;

/// Arranques sucios que disparan el modo seguro.
pub const THRESHOLD: u32 = 3;
/// Ventana en la que se cuentan esos arranques.
pub const WINDOW: Duration = Duration::minutes(10);
/// Tiempo sin caídas tras el cual el modo seguro se levanta solo.
pub const COOLDOWN: Duration = Duration::hours(1);

const KEY_CLEAN: &str = "clean_shutdown";
const KEY_COUNT: &str = "unclean_starts";
const KEY_FIRST: &str = "first_unclean_at";
const KEY_ENTERED: &str = "safe_mode_since";

/// Resultado de evaluar el arranque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub safe_mode: bool,
    pub unclean_starts: u32,
    pub reason: Option<String>,
}

fn read_u32(store: &Store, key: &str) -> u32 {
    store.meta(key).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn read_time(store: &Store, key: &str) -> Option<DateTime<Utc>> {
    store.meta(key).ok().flatten().and_then(|v| DateTime::parse_from_rfc3339(&v).ok()).map(|t| t.with_timezone(&Utc))
}

/// Registra este arranque y decide si el agente entra en modo seguro.
///
/// # Errors
/// Error al escribir en la base local.
pub fn on_start(store: &Store, now: DateTime<Utc>) -> anyhow::Result<Startup> {
    let clean = store.meta(KEY_CLEAN)?.as_deref() == Some("true");
    // El marcador se pone a false ahora: si el proceso muere sin apagado ordenado, el próximo
    // arranque lo verá.
    store.set_meta(KEY_CLEAN, "false")?;

    if clean {
        store.set_meta(KEY_COUNT, "0")?;
        store.set_meta(KEY_FIRST, "")?;
        return Ok(Startup { safe_mode: false, unclean_starts: 0, reason: None });
    }

    let first = read_time(store, KEY_FIRST);
    let within_window = first.is_some_and(|first| now - first <= WINDOW);
    let count = if within_window { read_u32(store, KEY_COUNT) + 1 } else { 1 };
    if !within_window {
        store.set_meta(KEY_FIRST, &now.to_rfc3339())?;
    }
    store.set_meta(KEY_COUNT, &count.to_string())?;

    // Si ya estaba en modo seguro, se mantiene hasta que pase el enfriamiento sin caídas nuevas.
    if let Some(since) = read_time(store, KEY_ENTERED)
        && now - since < COOLDOWN
    {
        return Ok(Startup {
            safe_mode: true,
            unclean_starts: count,
            reason: Some("the agent is still in safe mode after repeated crashes".to_owned()),
        });
    }

    if count >= THRESHOLD {
        store.set_meta(KEY_ENTERED, &now.to_rfc3339())?;
        return Ok(Startup {
            safe_mode: true,
            unclean_starts: count,
            reason: Some(format!("{count} unclean starts in {} minutes", WINDOW.num_minutes())),
        });
    }
    Ok(Startup { safe_mode: false, unclean_starts: count, reason: None })
}

/// Marca un apagado ordenado: el siguiente arranque no cuenta como caída.
///
/// # Errors
/// Error al escribir en la base local.
pub fn on_clean_shutdown(store: &Store) -> anyhow::Result<()> {
    store.set_meta(KEY_CLEAN, "true")?;
    store.set_meta(KEY_COUNT, "0")?;
    store.set_meta(KEY_FIRST, "")?;
    Ok(())
}

/// Levanta el modo seguro (lo pide el owner con `status --clear-safe-mode`).
///
/// # Errors
/// Error al escribir en la base local.
pub fn clear(store: &Store) -> anyhow::Result<()> {
    store.set_meta(KEY_ENTERED, "")?;
    store.set_meta(KEY_COUNT, "0")?;
    store.set_meta(KEY_FIRST, "")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("agent.db")).expect("store");
        (dir, store)
    }

    #[test]
    fn a_clean_shutdown_resets_the_counter() {
        let (_dir, store) = store();
        let now = Utc::now();
        assert!(!on_start(&store, now).expect("start").safe_mode);
        on_clean_shutdown(&store).expect("shutdown");
        let second = on_start(&store, now + Duration::seconds(30)).expect("start");
        assert_eq!(second.unclean_starts, 0);
        assert!(!second.safe_mode);
    }

    #[test]
    fn three_crashes_in_the_window_enter_safe_mode() {
        let (_dir, store) = store();
        let start = Utc::now();
        assert!(!on_start(&store, start).expect("1").safe_mode);
        assert!(!on_start(&store, start + Duration::minutes(1)).expect("2").safe_mode);
        let third = on_start(&store, start + Duration::minutes(2)).expect("3");
        assert!(third.safe_mode);
        assert_eq!(third.unclean_starts, 3);
        assert!(third.reason.expect("reason").contains("unclean starts"));
    }

    #[test]
    fn crashes_spread_over_time_do_not_trigger_it() {
        let (_dir, store) = store();
        let start = Utc::now();
        for minutes in [0, 30, 60] {
            let result = on_start(&store, start + Duration::minutes(minutes)).expect("start");
            assert!(!result.safe_mode, "a los {minutes} minutos");
            assert_eq!(result.unclean_starts, 1, "la ventana se reinicia");
        }
    }

    #[test]
    fn safe_mode_persists_until_cleared_or_cooled_down() {
        let (_dir, store) = store();
        let start = Utc::now();
        for minutes in [0, 1, 2] {
            on_start(&store, start + Duration::minutes(minutes)).expect("start");
        }
        // Un arranque más, aún dentro del enfriamiento: sigue en modo seguro.
        assert!(on_start(&store, start + Duration::minutes(20)).expect("start").safe_mode);
        // Pasada una hora sin entrar de nuevo, deja de aplicarse.
        assert!(
            !on_start(&store, start + Duration::minutes(2) + COOLDOWN + Duration::minutes(1)).expect("start").safe_mode
        );

        // Y el owner puede levantarlo explícitamente.
        for minutes in [0, 1, 2] {
            on_start(&store, start + Duration::hours(3) + Duration::minutes(minutes)).expect("start");
        }
        clear(&store).expect("clear");
        assert!(!on_start(&store, start + Duration::hours(3) + Duration::minutes(3)).expect("start").safe_mode);
    }
}
