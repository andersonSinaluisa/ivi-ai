//! Raíces permitidas y catálogo de comandos con nombre (docs/03-tools/filesystem-tools.md,
//! docs/03-tools/command-execution.md).
//!
//! Dos reglas que no se negocian: **sin raíces configuradas no se lee nada**, y el Cloud nunca envía
//! rutas ni líneas de comandos: elige entradas que el owner definió aquí.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Intérpretes y utilidades que convertirían un "comando con nombre" en ejecución arbitraria.
pub const FORBIDDEN_EXECUTABLES: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "bash.exe",
    "wsl.exe",
    "conhost.exe",
    "msbuild.exe",
    "installutil.exe",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedRoot {
    /// Ruta absoluta (admite `%VARIABLES%`, que expande la plataforma).
    pub path: String,
    #[serde(default = "yes")]
    pub list: bool,
    #[serde(default)]
    pub read: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemPolicy {
    /// Sin entradas, las tools de filesystem rechazan cualquier ruta.
    #[serde(default)]
    pub allowed_roots: Vec<AllowedRoot>,
    #[serde(default = "max_read")]
    pub max_read_bytes: usize,
    #[serde(default = "max_entries")]
    pub max_list_entries: usize,
    #[serde(default)]
    pub include_hidden: bool,
    /// Extensiones que nunca se leen aunque estén dentro de una raíz permitida.
    #[serde(default = "denied_extensions")]
    pub denied_extensions: Vec<String>,
}

impl Default for FilesystemPolicy {
    fn default() -> Self {
        Self {
            allowed_roots: Vec::new(),
            max_read_bytes: max_read(),
            max_list_entries: max_entries(),
            include_hidden: false,
            denied_extensions: denied_extensions(),
        }
    }
}

const fn max_read() -> usize {
    262_144
}
const fn max_entries() -> usize {
    500
}

fn denied_extensions() -> Vec<String> {
    ["pfx", "p12", "pem", "key", "kdbx", "ppk", "rdp", "ovpn", "jks", "keystore"].map(String::from).to_vec()
}

/// Parámetro de un comando con nombre: siempre tipado, nunca texto libre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandParam {
    /// Valor de una lista cerrada.
    Enum {
        values: Vec<String>,
    },
    /// Texto con patrón obligatorio y anclado (se valida al cargar la política).
    String {
        pattern: String,
    },
    Int {
        min: i64,
        max: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEntry {
    pub description: String,
    /// Ruta absoluta del ejecutable (admite `%VARIABLES%`).
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub params: BTreeMap<String, CommandParam>,
    #[serde(default = "r3")]
    pub risk: String,
    #[serde(default = "command_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "zero_exit")]
    pub expected_exit_codes: Vec<i32>,
    #[serde(default)]
    pub capture_output: bool,
    #[serde(default = "max_command_output")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub working_directory: Option<String>,
}

fn r3() -> String {
    "R3".to_owned()
}
const fn command_timeout() -> u64 {
    30_000
}
fn zero_exit() -> Vec<i32> {
    vec![0]
}
const fn max_command_output() -> usize {
    8192
}

/// Formato de un `commandId`: `letras.minúsculas` con puntos, guiones y dígitos (`dotnet.test`).
#[must_use]
pub fn is_valid_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-')
}

/// Comprueba una entrada del catálogo de comandos al cargar la política.
///
/// # Errors
/// Mensaje explicando qué regla incumple.
pub fn validate_command(id: &str, entry: &CommandEntry) -> Result<(), String> {
    if !is_valid_command_id(id) {
        return Err(format!("command id '{id}' must be lower-case with dots, dashes or underscores"));
    }
    let executable = Path::new(&entry.executable);
    let absolute = entry.executable.starts_with('%') || executable.is_absolute();
    if !absolute {
        return Err(format!("command '{id}': the executable must be an absolute path"));
    }
    let file_name = executable.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if FORBIDDEN_EXECUTABLES.iter().any(|f| *f == file_name) {
        return Err(format!("command '{id}': '{file_name}' turns a named command into arbitrary execution (ADR-008)"));
    }
    if !matches!(entry.risk.as_str(), "R1" | "R2" | "R3") {
        return Err(format!("command '{id}': risk must be R1, R2 or R3"));
    }
    if entry.timeout_ms == 0 || entry.timeout_ms > 600_000 {
        return Err(format!("command '{id}': timeout_ms must be in 1..=600000"));
    }
    if entry.max_output_bytes > 24 * 1024 {
        return Err(format!("command '{id}': max_output_bytes must be <= 24576"));
    }

    // Cada `{param}` de los argumentos debe estar declarado, y cada declarado debe usarse.
    let placeholders: Vec<String> = entry
        .args
        .iter()
        .filter_map(|arg| arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')).map(str::to_owned))
        .collect();
    for name in &placeholders {
        if !entry.params.contains_key(name) {
            return Err(format!("command '{id}': parameter '{name}' is used but not declared"));
        }
    }
    for (name, param) in &entry.params {
        if !placeholders.contains(name) {
            return Err(format!("command '{id}': parameter '{name}' is declared but never used"));
        }
        if let CommandParam::String { pattern } = param {
            if !pattern.starts_with('^') || !pattern.ends_with('$') {
                return Err(format!("command '{id}': the pattern of '{name}' must be anchored with ^ and $"));
            }
            Regex::new(pattern)
                .map_err(|error| format!("command '{id}': the pattern of '{name}' is invalid: {error}"))?;
        }
        if let CommandParam::Enum { values } = param
            && values.is_empty()
        {
            return Err(format!("command '{id}': the enum of '{name}' is empty"));
        }
    }
    Ok(())
}

/// Comprueba un valor contra la declaración de su parámetro y devuelve el texto del argumento.
///
/// El valor **nunca** se interpreta: se compara con una lista cerrada, con un patrón anclado o con
/// un rango, y solo entonces se convierte en un argumento completo.
///
/// # Errors
/// Mensaje explicando por qué el valor no vale.
pub fn validate_param(name: &str, param: &CommandParam, value: &serde_json::Value) -> Result<String, String> {
    match param {
        CommandParam::Enum { values } => {
            let text = value.as_str().ok_or_else(|| format!("'{name}' must be a string"))?;
            if values.iter().any(|allowed| allowed == text) {
                Ok(text.to_owned())
            } else {
                Err(format!("'{name}' is not one of the allowed values"))
            }
        }
        CommandParam::String { pattern } => {
            let text = value.as_str().ok_or_else(|| format!("'{name}' must be a string"))?;
            if text.len() > 512 {
                return Err(format!("'{name}' is too long"));
            }
            let regex = Regex::new(pattern).map_err(|_| format!("'{name}' has an invalid pattern"))?;
            if regex.is_match(text) {
                Ok(text.to_owned())
            } else {
                Err(format!("'{name}' does not match the allowed pattern"))
            }
        }
        CommandParam::Int { min, max } => {
            let number = value.as_i64().ok_or_else(|| format!("'{name}' must be an integer"))?;
            if number < *min || number > *max {
                return Err(format!("'{name}' must be in {min}..={max}"));
            }
            Ok(number.to_string())
        }
    }
}

/// Construye los argumentos del comando sustituyendo cada `{param}` por su valor validado.
///
/// La sustitución es de **argumento completo**: un valor nunca se concatena dentro de otro
/// argumento, así que no puede añadir opciones ni jugar con el entrecomillado.
///
/// # Errors
/// Si sobra o falta algún parámetro, o si alguno no supera su validación.
pub fn render_arguments(
    entry: &CommandEntry,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<String>, String> {
    for key in values.keys() {
        if !entry.params.contains_key(key) {
            return Err(format!("unknown parameter '{key}'"));
        }
    }
    let mut rendered = Vec::with_capacity(entry.args.len());
    for arg in &entry.args {
        if let Some(name) = arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
            let param = entry.params.get(name).ok_or_else(|| format!("unknown parameter '{name}'"))?;
            let value = values.get(name).ok_or_else(|| format!("missing parameter '{name}'"))?;
            rendered.push(validate_param(name, param, value)?);
        } else {
            rendered.push(arg.clone());
        }
    }
    Ok(rendered)
}

/// Formas de ruta que se rechazan **antes** de tocar el disco (WT-06, WT-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRejection {
    NotAbsolute,
    DeviceNamespace,
    UncPath,
    ReservedName,
    AlternateDataStream,
    ControlCharacters,
    TooLong,
    Traversal,
}

impl PathRejection {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotAbsolute => "the path must be absolute",
            Self::DeviceNamespace => "device namespaces (\\\\?\\, \\\\.\\) are not allowed",
            Self::UncPath => "UNC and network paths are not allowed",
            Self::ReservedName => "reserved device names (CON, NUL, COM1…) are not allowed",
            Self::AlternateDataStream => "alternate data streams are not allowed",
            Self::ControlCharacters => "the path contains control characters",
            Self::TooLong => "the path is too long",
            Self::Traversal => "the path contains '..'",
        }
    }
}

const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Validación puramente textual de una ruta recibida del Cloud.
///
/// # Errors
/// El motivo del rechazo; el llamante lo convierte en `INVALID_PAYLOAD`/`PATH_NOT_ALLOWED`.
pub fn check_path_shape(raw: &str) -> Result<PathBuf, PathRejection> {
    if raw.chars().count() > 1024 {
        return Err(PathRejection::TooLong);
    }
    if raw.chars().any(|c| c.is_control()) {
        return Err(PathRejection::ControlCharacters);
    }
    if raw.starts_with("\\\\?\\") || raw.starts_with("\\\\.\\") || raw.starts_with("//?/") || raw.starts_with("//./") {
        return Err(PathRejection::DeviceNamespace);
    }
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err(PathRejection::UncPath);
    }
    // Un `:` solo puede ser el de la unidad (`C:`); cualquier otro abre un flujo alternativo.
    if raw.match_indices(':').any(|(index, _)| index != 1) {
        return Err(PathRejection::AlternateDataStream);
    }
    let path = PathBuf::from(raw);
    let mut components = path.components();
    let absolute = matches!(components.next(), Some(Component::Prefix(_)))
        && matches!(components.next(), Some(Component::RootDir));
    if !absolute {
        return Err(PathRejection::NotAbsolute);
    }
    for component in path.components() {
        if let Component::Normal(part) = component {
            let name = part.to_string_lossy().to_lowercase();
            let stem = name.split('.').next().unwrap_or_default();
            if RESERVED_NAMES.contains(&stem) {
                return Err(PathRejection::ReservedName);
            }
        }
        if matches!(component, Component::ParentDir) {
            return Err(PathRejection::Traversal);
        }
    }
    Ok(path)
}

/// `true` si `path` está dentro de `root`, comparando **componente a componente** (así `C:\Docs2`
/// no cuenta como dentro de `C:\Docs`).
#[must_use]
pub fn is_within(path: &Path, root: &Path) -> bool {
    let mut path_components = path.components();
    for root_component in root.components() {
        let Some(candidate) = path_components.next() else { return false };
        let same =
            candidate.as_os_str().to_string_lossy().eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy());
        if !same {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_path_shapes() {
        let cases = [
            (r"..\secret.txt", PathRejection::NotAbsolute),
            (r"secret.txt", PathRejection::NotAbsolute),
            (r"C:\allowed\..\secret.txt", PathRejection::Traversal),
            (r"\\?\C:\secret.txt", PathRejection::DeviceNamespace),
            (r"\\.\PhysicalDrive0", PathRejection::DeviceNamespace),
            (r"\\server\share\x", PathRejection::UncPath),
            (r"C:\allowed\CON", PathRejection::ReservedName),
            (r"C:\allowed\nul.txt", PathRejection::ReservedName),
            (r"C:\allowed\a.txt:hidden", PathRejection::AlternateDataStream),
            ("C:\\allowed\\a\u{0}.txt", PathRejection::ControlCharacters),
        ];
        for (raw, expected) in cases {
            assert_eq!(check_path_shape(raw), Err(expected), "{raw}");
        }
        assert!(check_path_shape(&format!(r"C:\{}", "a".repeat(1100))).is_err());
        assert!(check_path_shape(r"C:\allowed\notes.txt").is_ok());
    }

    #[test]
    fn root_containment_compares_components() {
        let root = Path::new(r"C:\JarvisTest\allowed");
        assert!(is_within(Path::new(r"C:\JarvisTest\allowed\a.txt"), root));
        assert!(is_within(Path::new(r"c:\jarvistest\ALLOWED\sub\a.txt"), root));
        assert!(!is_within(Path::new(r"C:\JarvisTest\allowed2\a.txt"), root), "el prefijo textual no basta");
        assert!(!is_within(Path::new(r"C:\JarvisTest\other\a.txt"), root));
    }

    fn entry(executable: &str) -> CommandEntry {
        CommandEntry {
            description: "test".into(),
            executable: executable.into(),
            args: Vec::new(),
            params: BTreeMap::new(),
            risk: "R2".into(),
            timeout_ms: 5000,
            expected_exit_codes: vec![0],
            capture_output: false,
            max_output_bytes: 4096,
            working_directory: None,
        }
    }

    #[test]
    fn rejects_interpreters_as_named_commands() {
        for shell in
            ["C:\\Windows\\System32\\cmd.exe", "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"]
        {
            let error = validate_command("evil", &entry(shell)).expect_err("debe rechazarse");
            assert!(error.contains("arbitrary execution"), "{error}");
        }
        assert!(validate_command("network.flush-dns", &entry(r"%SystemRoot%\System32\ipconfig.exe")).is_ok());
    }

    #[test]
    fn parameters_must_be_declared_used_and_anchored() {
        let mut with_arg = entry(r"C:\tools\backup.exe");
        with_arg.args = vec!["--target".into(), "{target}".into()];
        assert!(validate_command("x", &with_arg).is_err(), "parámetro sin declarar");

        with_arg.params.insert("target".into(), CommandParam::String { pattern: "[a-z]+".into() });
        let error = validate_command("x", &with_arg).expect_err("patrón sin anclar");
        assert!(error.contains("anchored"), "{error}");

        with_arg.params.insert("target".into(), CommandParam::String { pattern: "^[a-z]+$".into() });
        assert!(validate_command("x", &with_arg).is_ok());

        with_arg.params.insert("unused".into(), CommandParam::Enum { values: vec!["a".into()] });
        assert!(validate_command("x", &with_arg).is_err(), "parámetro declarado y no usado");
    }

    #[test]
    fn filesystem_defaults_are_closed() {
        let policy = FilesystemPolicy::default();
        assert!(policy.allowed_roots.is_empty(), "sin raíces no se lee nada");
        assert!(!policy.include_hidden);
        assert!(policy.denied_extensions.iter().any(|e| e == "pem"));
    }
}
