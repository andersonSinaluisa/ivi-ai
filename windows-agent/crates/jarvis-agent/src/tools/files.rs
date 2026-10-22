//! Tools de filesystem de solo lectura (docs/03-tools/filesystem-tools.md).
//!
//! El orden de comprobaciones es el documentado y no se puede reordenar: forma de la ruta →
//! prefijo léxico contra las raíces → apertura sin seguir reparse points → **revalidación de la
//! ruta final del handle** → operaciones sobre ese handle. Sin raíces configuradas no se lee nada.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jarvis_policy::files::AllowedRoot;
use jarvis_policy::{check_path_shape, is_within};
use jarvis_tooling::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationSpec};
use jarvis_tooling::platform::{Opened, PlatformErrorKind};
use jarvis_tooling::{
    ErrorCode, ExecContext, ExecutionOutcome, ResourceLock, RiskLevel, Tool, ToolDescriptor, ToolError,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ToolContext;
use super::apps::descriptor;
use crate::logging::Sensitive;

/// Bytes que se miran para decidir si un archivo es binario.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Tope duro de contenido devuelto en una tarea (coincide con `max_output_bytes`).
const MAX_CONTENT_BYTES: usize = 24 * 1024;

/// Qué permiso de la raíz hace falta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    List,
    Read,
}

impl Access {
    const fn granted_by(self, root: &AllowedRoot) -> bool {
        match self {
            Self::List => root.list,
            Self::Read => root.read,
        }
    }

    const fn what(self) -> &'static str {
        match self {
            Self::List => "listed",
            Self::Read => "read",
        }
    }
}

/// Raíces permitidas ya expandidas (`%USERPROFILE%` → ruta real) para este acceso.
fn roots_for(ctx: &ToolContext, access: Access) -> Vec<PathBuf> {
    ctx.policy()
        .filesystem
        .allowed_roots
        .iter()
        .filter(|root| access.granted_by(root))
        .map(|root| PathBuf::from(ctx.api.expand_env(&root.path)))
        .collect()
}

fn not_allowed(access: Access) -> ToolError {
    ToolError::new(
        ErrorCode::PathNotAllowed,
        format!("the path is outside the roots that may be {} (see [filesystem] in policy.toml)", access.what()),
    )
}

/// Validación textual compartida por las dos tools: forma de la ruta y prefijo léxico contra las
/// raíces permitidas. No toca el disco, así que una ruta hostil se rechaza (`task.rejected`) sin
/// haber abierto nada.
fn validate_path(ctx: &ToolContext, raw: &str, access: Access) -> Result<String, ToolError> {
    let path =
        check_path_shape(raw).map_err(|rejection| ToolError::new(ErrorCode::PathNotAllowed, rejection.reason()))?;
    let roots = roots_for(ctx, access);
    if roots.is_empty() {
        return Err(ToolError::new(
            ErrorCode::PathNotAllowed,
            "no filesystem roots are configured: this tool is disabled until the owner adds some",
        ));
    }
    if !roots.iter().any(|root| is_within(&path, root)) {
        return Err(not_allowed(access));
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Abre la ruta con el sandbox y confirma que la **ruta real** sigue dentro de una raíz permitida.
///
/// Las raíces se vuelven a leer aquí: entre la validación y la ejecución puede haber pasado un
/// `policy reload`, y siempre manda la política del momento de actuar.
async fn open_within(ctx: &ToolContext, path: &str, access: Access) -> Result<Opened, ToolError> {
    let roots = roots_for(ctx, access);
    let requested = PathBuf::from(path);
    if roots.is_empty() || !roots.iter().any(|root| is_within(&requested, root)) {
        return Err(not_allowed(access));
    }

    let target = requested.clone();
    let opened = ctx.blocking(move |api| api.open_sandboxed(&target)).await.map_err(|error| match error.code {
        ErrorCode::AppNotFound => ToolError::new(ErrorCode::FileNotFound, "the path does not exist"),
        ErrorCode::AccessDenied => ToolError::new(ErrorCode::PathNotAllowed, error.message),
        _ => error,
    })?;

    // Comprobación que manda: la ruta real del handle (resuelve enlaces intermedios y nombres 8.3).
    if !roots.iter().any(|root| is_within(&opened.final_path, root)) {
        return Err(not_allowed(access));
    }
    Ok(opened)
}

/// Clave de lock: la ruta canónica en minúsculas.
fn fs_lock(path: &str) -> ResourceLock {
    ResourceLock::shared(format!("FS:{}", path.to_lowercase()))
}

fn modified_at(time: Option<std::time::SystemTime>) -> Option<String> {
    time.map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

// ---- list_directory ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListInput {
    path: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_limit() -> usize {
    100
}

pub struct ListDirectory {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl ListDirectory {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "list_directory",
                "Lists the entries of a directory inside the allowed roots (not recursive)",
                RiskLevel::R1,
                vec!["filesystem.list"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["path"],
                    "properties": {
                        "path": {"type": "string", "minLength": 3, "maxLength": 1024},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                    }
                }),
                json!({"type": "object", "properties": {"path": {"type": "string"}, "entries": {"type": "array"}, "truncated": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(3), max: Duration::from_secs(10) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                // Los nombres de archivo pueden ser personales: no se loguean y se redactan tras settled.
                Sensitivity::Output,
                false,
            ),
        }
    }
}

#[async_trait]
impl Tool for ListDirectory {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: ListInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if !(1..=500).contains(&parsed.limit) {
            return Err(ToolError::invalid("limit must be in 1..=500"));
        }
        Ok(json!({ "path": validate_path(&self.ctx, &parsed.path, Access::List)?, "limit": parsed.limit }))
    }

    fn resources(&self, input: &Value) -> Vec<ResourceLock> {
        input.get("path").and_then(Value::as_str).map(|path| vec![fs_lock(path)]).unwrap_or_default()
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let path = input.get("path").and_then(Value::as_str).ok_or_else(|| ToolError::invalid("missing path"))?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        let policy = self.ctx.policy();
        let limit = limit.min(policy.filesystem.max_list_entries);

        let opened = open_within(&self.ctx, path, Access::List).await?;
        if !opened.is_dir {
            return Err(ToolError::invalid("the path is a file, not a directory"));
        }

        let include_hidden = policy.filesystem.include_hidden;
        let entry = opened.entry;
        // La lectura del directorio es bloqueante: fuera del runtime async.
        let listed = tokio::task::spawn_blocking(move || entry.list(limit + 1))
            .await
            .map_err(|_| ToolError::new(ErrorCode::SystemError, "the directory listing panicked"))?
            .map_err(super::platform_error)?;

        let mut visible: Vec<_> = listed.into_iter().filter(|e| include_hidden || !e.hidden).collect();
        visible.sort_by(|a, b| (b.is_dir, a.name.to_lowercase()).cmp(&(a.is_dir, b.name.to_lowercase())));
        let truncated = visible.len() > limit;
        visible.truncate(limit);

        let entries: Vec<Value> = visible
            .into_iter()
            .map(|e| {
                json!({
                    "name": Sensitive::new(e.name).into_inner(),
                    "kind": if e.is_dir { "dir" } else { "file" },
                    "size": (!e.is_dir).then_some(e.size),
                    "modifiedAt": modified_at(e.modified),
                })
            })
            .collect();

        Ok(ExecutionOutcome {
            output: json!({
                "path": opened.final_path.to_string_lossy(),
                "entries": entries,
                "truncated": truncated,
            }),
            details: Some(json!({ "method": "directory_handle" })),
            ..Default::default()
        })
    }
}

// ---- read_file --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadInput {
    path: String,
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_max_bytes")]
    max_bytes: usize,
}

const fn default_max_bytes() -> usize {
    16 * 1024
}

pub struct ReadFile {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
}

impl ReadFile {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "read_file",
                "Reads a window of a UTF-8 text file inside the allowed roots",
                RiskLevel::R1,
                vec!["filesystem.read"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["path"],
                    "properties": {
                        "path": {"type": "string", "minLength": 3, "maxLength": 1024},
                        "offset": {"type": "integer", "minimum": 0, "default": 0},
                        "maxBytes": {"type": "integer", "minimum": 1, "maximum": 24576, "default": 16384}
                    }
                }),
                json!({"type": "object", "properties": {"path": {"type": "string"}, "encoding": {"type": "string"}, "content": {"type": "string"}, "size": {"type": "integer"}, "sha256": {"type": "string"}, "truncated": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(3), max: Duration::from_secs(10) },
                CancellationMode::None,
                VerificationSpec::not_applicable(),
                // Ruta y contenido: ninguno de los dos se loguea, ambos se redactan tras settled.
                Sensitivity::InputAndOutput,
                false,
            ),
        }
    }
}

/// `true` si la extensión (o el nombre `id_rsa` sin extensión) está vetada aunque la raíz la permita.
fn is_denied_file(path: &Path, denied: &[String]) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some(extension) = path.extension().map(|e| e.to_string_lossy().to_lowercase())
        && denied.iter().any(|denied| denied.eq_ignore_ascii_case(&extension))
    {
        return true;
    }
    // Claves SSH: `id_rsa`, `id_ed25519`… sin extensión.
    path.extension().is_none() && name.starts_with("id_")
}

/// Decodifica el trozo leído. UTF-16 con BOM se convierte; binario se rechaza.
fn decode(window: &[u8], sniff: &[u8]) -> Result<(String, &'static str), ToolError> {
    if sniff.starts_with(&[0xFF, 0xFE]) || sniff.starts_with(&[0xFE, 0xFF]) {
        let big_endian = sniff.starts_with(&[0xFE, 0xFF]);
        let body = if window.len() >= 2 && (window.starts_with(&[0xFF, 0xFE]) || window.starts_with(&[0xFE, 0xFF])) {
            &window[2..]
        } else {
            window
        };
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|pair| {
                if big_endian { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) }
            })
            .collect();
        let text = String::from_utf16(&units)
            .map_err(|_| ToolError::new(ErrorCode::BinaryFile, "the file is not valid UTF-16 text"))?;
        return Ok((text, "utf-16"));
    }
    if sniff.contains(&0) {
        return Err(ToolError::new(ErrorCode::BinaryFile, "the file looks binary (NUL bytes) and is not returned"));
    }
    // La ventana puede cortar un carácter multibyte: se descarta la cola incompleta.
    let mut end = window.len();
    while end > 0 && (window[end - 1] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    let candidate = if end > 0 && window[end - 1] >= 0x80 { end - 1 } else { end };
    match std::str::from_utf8(&window[..end]) {
        Ok(text) => Ok((text.to_owned(), "utf-8")),
        Err(_) => std::str::from_utf8(&window[..candidate])
            .map(|text| (text.to_owned(), "utf-8"))
            .map_err(|_| ToolError::new(ErrorCode::BinaryFile, "the file is not valid UTF-8 text")),
    }
}

#[async_trait]
impl Tool for ReadFile {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: ReadInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        if parsed.max_bytes == 0 || parsed.max_bytes > MAX_CONTENT_BYTES {
            return Err(ToolError::invalid("maxBytes must be in 1..=24576"));
        }
        let path = validate_path(&self.ctx, &parsed.path, Access::Read)?;
        let policy = self.ctx.policy();
        if is_denied_file(Path::new(&path), &policy.filesystem.denied_extensions) {
            return Err(ToolError::new(ErrorCode::PathNotAllowed, "this file type is never returned (credentials)"));
        }
        Ok(json!({ "path": path, "offset": parsed.offset, "maxBytes": parsed.max_bytes }))
    }

    fn resources(&self, input: &Value) -> Vec<ResourceLock> {
        input.get("path").and_then(Value::as_str).map(|path| vec![fs_lock(path)]).unwrap_or_default()
    }

    async fn execute(&self, input: &Value, _ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let path = input.get("path").and_then(Value::as_str).ok_or_else(|| ToolError::invalid("missing path"))?;
        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let max_bytes = input.get("maxBytes").and_then(Value::as_u64).unwrap_or(16384) as usize;
        let policy = self.ctx.policy();

        let opened = open_within(&self.ctx, path, Access::Read).await?;
        if opened.is_dir {
            return Err(ToolError::invalid("the path is a directory, not a file"));
        }
        // La ruta real puede tener otra extensión que la pedida (enlaces intermedios ya rechazados,
        // pero el nombre canónico manda).
        if is_denied_file(&opened.final_path, &policy.filesystem.denied_extensions) {
            return Err(ToolError::new(ErrorCode::PathNotAllowed, "this file type is never returned (credentials)"));
        }
        if !policy.filesystem.include_hidden && opened.hidden {
            return Err(ToolError::new(ErrorCode::PathNotAllowed, "hidden and system files are excluded"));
        }
        if offset > opened.size {
            return Err(ToolError::invalid("offset is past the end of the file"));
        }

        let size = opened.size;
        let max_read = policy.filesystem.max_read_bytes;
        let entry = opened.entry;
        // Lecturas sobre el handle ya validado, fuera del runtime async.
        let (window, sniff, digest) = tokio::task::spawn_blocking(move || {
            let window = entry.read(offset, max_bytes)?;
            let sniff = if offset == 0 { window.clone() } else { entry.read(0, BINARY_SNIFF_BYTES)? };
            // El hash es del archivo completo y solo si cabe en el presupuesto de lectura.
            let digest = if size <= max_read as u64 {
                let whole = entry.read(0, usize::try_from(size).unwrap_or(max_read))?;
                Some(format!("{:x}", Sha256::digest(&whole)))
            } else {
                None
            };
            Ok((window, sniff, digest))
        })
        .await
        .map_err(|_| ToolError::new(ErrorCode::SystemError, "the file read panicked"))?
        .map_err(|error: jarvis_tooling::platform::PlatformError| match error.kind {
            PlatformErrorKind::NotFound => {
                ToolError::new(ErrorCode::FileNotFound, "the file disappeared while reading")
            }
            _ => ToolError::new(ErrorCode::ExecutionFailed, error.message),
        })?;

        let read_bytes = window.len() as u64;
        let (content, encoding) = decode(&window, &sniff[..sniff.len().min(BINARY_SNIFF_BYTES)])?;
        // El contenido es sensible: se envuelve para que no pueda acabar en un log por descuido.
        let content = Sensitive::new(content);

        Ok(ExecutionOutcome {
            output: json!({
                "path": opened.final_path.to_string_lossy(),
                "encoding": encoding,
                "content": content.expose(),
                "size": size,
                "sha256": digest,
                "truncated": offset + read_bytes < size,
            }),
            details: Some(json!({ "method": "file_handle", "bytesRead": read_bytes })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_paths_never_reach_the_disk() {
        // La forma de la ruta se rechaza antes que nada, sin política ni disco de por medio.
        for raw in [
            r"..\secret.txt",
            r"C:\allowed\..\..\Windows\System32\config\SAM",
            r"\\server\share\file.txt",
            r"\\?\C:\secret.txt",
            r"C:\allowed\file.txt:stream",
            r"C:\allowed\NUL",
            "relative.txt",
        ] {
            assert!(check_path_shape(raw).is_err(), "debe rechazarse: {raw}");
        }
        assert!(check_path_shape(r"C:\allowed\notes.txt").is_ok());
    }

    #[test]
    fn credential_files_are_denied_by_extension_or_name() {
        let denied: Vec<String> = ["pfx", "pem", "kdbx", "key"].map(String::from).to_vec();
        assert!(is_denied_file(Path::new(r"C:\docs\cert.PFX"), &denied));
        assert!(is_denied_file(Path::new(r"C:\docs\vault.kdbx"), &denied));
        assert!(is_denied_file(Path::new(r"C:\users\x\.ssh\id_rsa"), &denied));
        assert!(!is_denied_file(Path::new(r"C:\docs\notes.txt"), &denied));
    }

    #[test]
    fn binary_content_is_rejected_and_text_decoded() {
        let binary = [0x50, 0x4B, 0x03, 0x04, 0x00, 0x01];
        assert_eq!(decode(&binary, &binary).expect_err("binario").code, ErrorCode::BinaryFile);

        let (text, encoding) = decode(b"hola", b"hola").expect("utf-8");
        assert_eq!((text.as_str(), encoding), ("hola", "utf-8"));

        // UTF-16LE con BOM: "hi".
        let utf16 = [0xFF, 0xFE, b'h', 0x00, b'i', 0x00];
        let (text, encoding) = decode(&utf16, &utf16).expect("utf-16");
        assert_eq!((text.as_str(), encoding), ("hi", "utf-16"));
    }

    #[test]
    fn truncated_multibyte_characters_are_dropped_instead_of_corrupting_the_output() {
        // "añ" cortado a la mitad del segundo carácter.
        let full = "añ".as_bytes().to_vec();
        let cut = &full[..full.len() - 1];
        let (text, _) = decode(cut, cut).expect("decode");
        assert_eq!(text, "a", "se devuelve solo lo que es texto completo");
    }
}
