//! Catálogo local de aplicaciones (docs/03-tools/tool-registry.md#catálogo-de-aplicaciones-appid).
//!
//! El Cloud nunca envía rutas: envía un identificador que se resuelve **solo** contra este catálogo.
//! La resolución acepta el `appId`, el nombre visible o un alias (`chrome`, `Chrome`, `Google Chrome`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LaunchSpec {
    /// Ejecutable registrado en `App Paths` (HKCU, luego HKLM).
    AppPaths { name: String },
    /// App empaquetada (MSIX/Store) por AUMID.
    Aumid { id: String },
    /// Ruta absoluta (admite `%VARIABLES%`); debe estar en una carpeta no escribible por otros usuarios.
    Path { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppEntry {
    pub display_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub launch: LaunchSpec,
    /// Nombres de ejecutable que identifican la app en ejecución (comparación sin mayúsculas).
    pub process_names: Vec<String>,
    /// Tiempo máximo para que aparezca la ventana tras lanzar.
    #[serde(default)]
    pub verify_timeout_ms: Option<u64>,
    /// Permite `close_app` sobre esta entrada (falso para `explorer`).
    #[serde(default = "yes")]
    pub allow_close: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("appId must be 1-64 characters: letters, digits, spaces, '-', '_' or '.'")]
    Invalid,
    #[error("application '{0}' is not in the local catalog")]
    NotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppCatalog {
    entries: BTreeMap<String, AppEntry>,
}

/// Normaliza para comparar: minúsculas, sin espacios, guiones, guiones bajos ni puntos.
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, ' ' | '-' | '_' | '.')).flat_map(char::to_lowercase).collect()
}

/// Formato de `appId`: `[a-z0-9_-]{1,64}`.
#[must_use]
pub fn is_valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Formato aceptado en el input remoto: nunca rutas ni metacaracteres.
fn is_acceptable_query(text: &str) -> bool {
    let count = text.chars().count();
    (1..=64).contains(&count)
        && text.trim() == text
        && text.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        && !text.contains("..")
}

impl AppCatalog {
    #[must_use]
    pub const fn new(entries: BTreeMap<String, AppEntry>) -> Self {
        Self { entries }
    }

    /// Entradas por defecto (docs/03-tools/tool-registry.md).
    #[must_use]
    pub fn defaults() -> Self {
        let mut entries = BTreeMap::new();
        let mut add =
            |id: &str, display: &str, aliases: &[&str], launch: LaunchSpec, procs: &[&str], allow_close: bool| {
                entries.insert(
                    id.to_owned(),
                    AppEntry {
                        display_name: display.to_owned(),
                        aliases: aliases.iter().map(|s| (*s).to_owned()).collect(),
                        launch,
                        process_names: procs.iter().map(|s| (*s).to_owned()).collect(),
                        verify_timeout_ms: None,
                        allow_close,
                    },
                );
            };
        add(
            "chrome",
            "Google Chrome",
            &["google chrome"],
            LaunchSpec::AppPaths { name: "chrome.exe".into() },
            &["chrome.exe"],
            true,
        );
        add(
            "edge",
            "Microsoft Edge",
            &["msedge", "microsoft edge"],
            LaunchSpec::AppPaths { name: "msedge.exe".into() },
            &["msedge.exe"],
            true,
        );
        add(
            "notepad",
            "Notepad",
            &["bloc de notas"],
            LaunchSpec::Path { path: r"%SystemRoot%\System32\notepad.exe".into() },
            &["notepad.exe"],
            true,
        );
        add(
            "calculator",
            "Calculator",
            &["calc", "calculadora"],
            LaunchSpec::Aumid { id: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into() },
            &["CalculatorApp.exe"],
            true,
        );
        add(
            "explorer",
            "File Explorer",
            &["file explorer", "explorador"],
            LaunchSpec::Path { path: r"%SystemRoot%\explorer.exe".into() },
            &["explorer.exe"],
            false,
        );
        Self { entries }
    }

    /// Superpone entradas del owner sobre las actuales (misma clave = reemplazo).
    pub fn extend(&mut self, other: BTreeMap<String, AppEntry>) {
        self.entries.extend(other);
    }

    #[must_use]
    pub fn get(&self, app_id: &str) -> Option<&AppEntry> {
        self.entries.get(app_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &AppEntry)> {
        self.entries.iter()
    }

    /// Resuelve lo que pidió el solicitante a un `appId` del catálogo.
    ///
    /// # Errors
    /// [`ResolveError::Invalid`] si el texto tiene un formato no permitido (rutas, metacaracteres);
    /// [`ResolveError::NotFound`] si no coincide con ninguna entrada.
    pub fn resolve(&self, query: &str) -> Result<(&str, &AppEntry), ResolveError> {
        if !is_acceptable_query(query) {
            return Err(ResolveError::Invalid);
        }
        if let Some((id, entry)) = self.entries.get_key_value(query) {
            return Ok((id.as_str(), entry));
        }
        let wanted = normalize(query);
        self.entries
            .iter()
            .find(|(id, entry)| {
                normalize(id) == wanted
                    || normalize(&entry.display_name) == wanted
                    || entry.aliases.iter().any(|a| normalize(a) == wanted)
            })
            .map(|(id, entry)| (id.as_str(), entry))
            .ok_or_else(|| ResolveError::NotFound(query.to_owned()))
    }

    /// `appId` de un proceso por nombre de ejecutable (para `list_running_apps`).
    #[must_use]
    pub fn app_for_process(&self, process_name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, e)| e.process_names.iter().any(|p| p.eq_ignore_ascii_case(process_name)))
            .map(|(id, _)| id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ids_display_names_and_aliases() {
        let catalog = AppCatalog::defaults();
        for query in ["chrome", "Chrome", "Google Chrome", "google-chrome", "GOOGLE CHROME"] {
            assert_eq!(catalog.resolve(query).map(|(id, _)| id), Ok("chrome"), "{query}");
        }
        assert_eq!(catalog.resolve("calc").map(|(id, _)| id), Ok("calculator"));
        assert_eq!(catalog.resolve("Notepad").map(|(id, _)| id), Ok("notepad"));
    }

    #[test]
    fn rejects_paths_and_metacharacters() {
        let catalog = AppCatalog::defaults();
        for query in [
            "",
            "../../cmd",
            r"C:\Windows\System32\cmd.exe",
            "chrome & calc",
            "chrome; calc",
            " chrome",
            "chrome\u{0}",
            "notepad.exe/../x",
            &"a".repeat(65),
        ] {
            assert_eq!(catalog.resolve(query), Err(ResolveError::Invalid), "{query:?}");
        }
        assert_eq!(catalog.resolve("cmd"), Err(ResolveError::NotFound("cmd".into())));
        assert!(matches!(catalog.resolve("format-disk"), Err(ResolveError::NotFound(_))));
    }

    #[test]
    fn maps_processes_to_apps_case_insensitively() {
        let catalog = AppCatalog::defaults();
        assert_eq!(catalog.app_for_process("Chrome.EXE"), Some("chrome"));
        assert_eq!(catalog.app_for_process("svchost.exe"), None);
    }
}
