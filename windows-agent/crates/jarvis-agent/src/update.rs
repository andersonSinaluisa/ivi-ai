//! Actualizador firmado con rollback (HU-WIN-063, mitiga WT-13).
//!
//! Tres barreras antes de que un binario llegue a ejecutarse, y ninguna la controla el Cloud:
//!
//! 1. el **manifiesto** va firmado con una clave Ed25519 *offline* cuya parte pública se compila
//!    dentro del agente (un Cloud comprometido no puede distribuir binarios);
//! 2. el binario descargado tiene que cuadrar en tamaño y **SHA-256** con lo que dice el manifiesto;
//! 3. y tiene que tener **firma Authenticode incrustada** válida según el propio Windows.
//!
//! Después, la versión nueva se instala **al lado** de la anterior (`versions\<semver>\`) y solo se
//! da por buena cuando arranca y conecta; si no, se vuelve sola a la anterior.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use jarvis_protocol::signature::{self, PublicKey, Signature};
use jarvis_state::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Esquema del manifiesto (versiona el formato y entra en la cadena firmada).
pub const MANIFEST_SCHEMA: &str = "jarvis.update-manifest/1";
/// Tope de tamaño del manifiesto y del binario.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_BINARY_BYTES: usize = 128 * 1024 * 1024;
/// Intentos de arranque de una versión nueva antes de volver a la anterior.
pub const MAX_UPDATE_ATTEMPTS: u32 = 3;
/// Cuánto tiene la versión nueva para arrancar y conectar antes de considerarse fallida.
pub const HEALTH_WINDOW_SECONDS: u64 = 120;

const META_PENDING: &str = "update.pending";

/// Clave pública offline del proyecto, compilada en el binario.
///
/// Las builds de release la reciben en `JARVIS_UPDATE_PUBLIC_KEY` (32 bytes Ed25519 en base64).
/// Una build sin clave **no puede actualizarse**: es preferible a aceptar cualquier manifiesto.
pub const EMBEDDED_PUBLIC_KEY: Option<&str> = option_env!("JARVIS_UPDATE_PUBLIC_KEY");

/// Claves con las que se acepta un manifiesto.
///
/// # Errors
/// Si la build no trae clave o la que trae no es válida.
pub fn release_keys() -> anyhow::Result<Vec<PublicKey>> {
    let encoded = EMBEDDED_PUBLIC_KEY.ok_or_else(|| {
        anyhow::anyhow!(
            "this build has no update key embedded: updates are disabled. \
             Install the new version manually with `jarvis-agent install`."
        )
    })?;
    let key = PublicKey::from_base64("release", encoded)
        .map_err(|e| anyhow::anyhow!("the embedded update key is invalid: {e}"))?;
    Ok(vec![key])
}

// ---- versiones ---------------------------------------------------------------------

/// SemVer reducido: `major.minor.patch` con prerelease opcional, que ordena **antes** que la final.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
    /// `None` = versión final. `Some` = prerelease (`1.0.0-rc.1` < `1.0.0`).
    prerelease: Option<String>,
}

impl Version {
    /// # Errors
    /// Si el texto no es `major.minor.patch[-prerelease]`.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let (core, prerelease) = text.split_once('-').map_or((text, None), |(c, p)| (c, Some(p.to_owned())));
        let mut parts = core.split('.');
        let mut number = |what: &str| -> anyhow::Result<u32> {
            parts
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("invalid version '{text}': missing {what}"))
        };
        let (major, minor, patch) = (number("major")?, number("minor")?, number("patch")?);
        if parts.next().is_some() {
            anyhow::bail!("invalid version '{text}': too many components");
        }
        if prerelease
            .as_ref()
            .is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        {
            anyhow::bail!("invalid version '{text}': malformed prerelease");
        }
        Ok(Self { major, minor, patch, prerelease })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(f, "-{prerelease}")?;
        }
        Ok(())
    }
}

/// Orden: primero el núcleo; con el mismo núcleo, una prerelease va **antes** que la final.
impl Version {
    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        let core = (self.major, self.minor, self.patch);
        let other_core = (other.major, other.minor, other.patch);
        if core != other_core {
            return core > other_core;
        }
        match (&self.prerelease, &other.prerelease) {
            (None, Some(_)) => true,
            (Some(_), None) | (None, None) => false,
            (Some(mine), Some(theirs)) => mine > theirs,
        }
    }
}

// ---- manifiesto --------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: String,
    pub version: String,
    pub channel: String,
    /// URL `https` del ejecutable.
    pub url: String,
    /// SHA-256 del ejecutable, en hexadecimal.
    pub sha256: String,
    pub size: u64,
    pub released_at: DateTime<Utc>,
    /// Versión mínima desde la que se puede saltar a esta (migraciones).
    #[serde(default)]
    pub min_upgradable_from: Option<String>,
    pub signature: Signature,
}

impl Manifest {
    /// Cadena canónica que firma la clave offline.
    #[must_use]
    pub fn signed_message(&self) -> String {
        format!(
            "{MANIFEST_SCHEMA}\nversion={}\nchannel={}\nurl={}\nsha256={}\nsize={}\nreleasedAt={}",
            self.version,
            self.channel,
            self.url,
            self.sha256.to_lowercase(),
            self.size,
            self.released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        )
    }
}

/// Actualización aceptada: qué descargar y adónde va.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub version: Version,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// Comprueba un manifiesto contra la versión actual y el canal configurado.
///
/// # Errors
/// Firma inválida, canal distinto, versión igual o anterior (anti-downgrade), salto no permitido o
/// campos fuera de rango. Ninguna de estas situaciones descarga nada.
pub fn check(manifest: &Manifest, current: &str, channel: &str, keys: &[PublicKey]) -> anyhow::Result<Plan> {
    if manifest.schema != MANIFEST_SCHEMA {
        anyhow::bail!("unsupported manifest schema '{}'", manifest.schema);
    }
    // La firma primero: el resto de campos no merece confianza hasta que cuadre.
    signature::verify(&manifest.signed_message(), &manifest.signature, keys)
        .map_err(|e| anyhow::anyhow!("the update manifest is not properly signed: {e}"))?;

    if manifest.channel != channel {
        anyhow::bail!("the manifest is for channel '{}' and this agent follows '{channel}'", manifest.channel);
    }
    let version = Version::parse(&manifest.version)?;
    let current = Version::parse(current)?;
    if !version.is_newer_than(&current) {
        anyhow::bail!("{version} is not newer than the installed {current} (downgrades are not allowed)");
    }
    if let Some(minimum) = &manifest.min_upgradable_from {
        let minimum = Version::parse(minimum)?;
        if minimum.is_newer_than(&current) {
            anyhow::bail!("upgrading to {version} requires at least {minimum}; install that one first");
        }
    }
    if manifest.size == 0 || manifest.size > MAX_BINARY_BYTES as u64 {
        anyhow::bail!("the manifest declares an implausible size ({} bytes)", manifest.size);
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("the manifest declares a malformed sha256");
    }
    if !manifest.url.starts_with("https://") {
        anyhow::bail!("the binary URL must be https");
    }
    Ok(Plan { version, url: manifest.url.clone(), sha256: manifest.sha256.to_lowercase(), size: manifest.size })
}

/// Comprueba el binario descargado contra el plan.
///
/// # Errors
/// Si el tamaño o el hash no cuadran con el manifiesto firmado.
pub fn check_binary(plan: &Plan, bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() as u64 != plan.size {
        anyhow::bail!("the downloaded file has {} bytes and the manifest says {}", bytes.len(), plan.size);
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    if digest != plan.sha256 {
        anyhow::bail!("the downloaded file does not match the sha256 of the manifest");
    }
    Ok(())
}

/// Carpeta de una versión instalada.
#[must_use]
pub fn version_dir(programs: &Path, version: &Version) -> PathBuf {
    programs.join("versions").join(version.to_string())
}

/// Escribe el binario ya verificado en `versions\<semver>\jarvis-agent.exe`.
///
/// # Errors
/// Si no se puede escribir.
pub fn stage(programs: &Path, plan: &Plan, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let directory = version_dir(programs, &plan.version);
    std::fs::create_dir_all(&directory)?;
    let target = directory.join("jarvis-agent.exe");
    // Escritura a un temporal y renombrado: nunca queda un exe a medias con nombre definitivo.
    let temporary = directory.join("jarvis-agent.exe.part");
    std::fs::write(&temporary, bytes)?;
    let _ = std::fs::remove_file(&target);
    std::fs::rename(&temporary, &target)?;
    Ok(target)
}

// ---- estado del cambio de versión --------------------------------------------------

/// Actualización aplicada pero todavía no confirmada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// Versión que se está probando.
    pub version: String,
    /// Ejecutable al que se vuelve si esta no funciona.
    pub previous_executable: PathBuf,
    /// Versión anterior (para los mensajes).
    pub previous_version: String,
    /// Arranques ya intentados con la versión nueva.
    pub attempts: u32,
    pub started_at: DateTime<Utc>,
}

/// Lee la actualización pendiente de confirmación.
///
/// # Errors
/// Error de la base local.
pub fn pending(store: &Store) -> anyhow::Result<Option<Pending>> {
    let Some(raw) = store.meta(META_PENDING)? else { return Ok(None) };
    match serde_json::from_str(&raw) {
        Ok(pending) => Ok(Some(pending)),
        Err(error) => {
            // Un registro ilegible no puede bloquear el arranque: se descarta avisando.
            tracing::warn!(%error, "unreadable update record: discarded");
            store.set_meta(META_PENDING, "")?;
            Ok(None)
        }
    }
}

/// Registra que se ha activado una versión nueva y queda pendiente de confirmarse.
///
/// # Errors
/// Error de la base local.
pub fn mark_pending(store: &Store, pending: &Pending) -> anyhow::Result<()> {
    store.set_meta(META_PENDING, &serde_json::to_string(pending)?)?;
    Ok(())
}

/// Suma un intento de arranque de la versión nueva y devuelve el estado actualizado.
///
/// # Errors
/// Error de la base local.
pub fn record_attempt(store: &Store, mut pending: Pending) -> anyhow::Result<Pending> {
    pending.attempts += 1;
    mark_pending(store, &pending)?;
    Ok(pending)
}

/// Da por buena la versión nueva: deja de haber nada pendiente.
///
/// # Errors
/// Error de la base local.
pub fn confirm(store: &Store) -> anyhow::Result<()> {
    store.set_meta(META_PENDING, "")?;
    Ok(())
}

/// `true` si esta actualización ya agotó sus intentos y toca volver atrás.
#[must_use]
pub const fn should_roll_back(pending: &Pending) -> bool {
    pending.attempts >= MAX_UPDATE_ATTEMPTS
}

// ---- orquestación (solo Windows) ---------------------------------------------------

/// Ejecuta el flujo completo: manifiesto → comprobaciones → descarga → verificación → instalación.
///
/// No reinicia nada por su cuenta: deja la versión nueva registrada como pendiente y devuelve su
/// ruta, para que el llamante decida cuándo cambiar (docs/05-windows/startup-model.md#actualizaciones).
///
/// # Errors
/// Cualquier comprobación que falle; ninguna deja el agente a medio actualizar.
#[cfg(windows)]
pub fn run(
    paths: &crate::paths::Paths,
    config: &crate::config::UpdateConfig,
    manifest_file: Option<&Path>,
    only_check: bool,
) -> anyhow::Result<Option<PathBuf>> {
    use jarvis_windows::authenticode::{self, Trust};

    let keys = release_keys()?;
    let raw = match manifest_file {
        // Archivo local: para equipos sin salida a internet; lo elige el owner, no el Cloud.
        Some(path) => std::fs::read(path)?,
        None => {
            let url = config
                .manifest_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("set update.manifest_url in config.toml or pass --manifest-file"))?;
            jarvis_windows::http::get(url, MAX_MANIFEST_BYTES)
                .map_err(|e| anyhow::anyhow!("cannot fetch the manifest: {e}"))?
        }
    };
    let manifest: Manifest = serde_json::from_slice(&raw).map_err(|e| anyhow::anyhow!("malformed manifest: {e}"))?;
    let plan = check(&manifest, crate::cloud::AGENT_VERSION, &config.channel, &keys)?;

    println!("Update available: {} -> {}", crate::cloud::AGENT_VERSION, plan.version);
    if only_check {
        return Ok(None);
    }

    let bytes = jarvis_windows::http::get(&plan.url, MAX_BINARY_BYTES)
        .map_err(|e| anyhow::anyhow!("cannot download the new version: {e}"))?;
    check_binary(&plan, &bytes)?;

    let programs = crate::install::programs_dir()?;
    let staged = stage(&programs, &plan, &bytes)?;

    // Authenticode sobre el archivo ya escrito: si no lo acepta Windows, no se queda en disco.
    match authenticode::verify(&staged) {
        Ok(Trust::Valid) => {}
        Ok(other) => {
            let _ = std::fs::remove_file(&staged);
            anyhow::bail!("the downloaded binary is not properly signed ({other:?}): update aborted");
        }
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            anyhow::bail!("cannot verify the signature of the downloaded binary: {error}");
        }
    }

    // Copia de la base antes de que una versión nueva la migre (AC5 de HU-WIN-063).
    backup_database(paths, &plan.version)?;
    println!("New version staged at {}", staged.display());
    Ok(Some(staged))
}

/// Copia `agent.db` antes de cambiar de versión. Sin base todavía no hay nada que copiar.
#[cfg(windows)]
fn backup_database(paths: &crate::paths::Paths, version: &Version) -> anyhow::Result<()> {
    let database = paths.database();
    if !database.is_file() {
        return Ok(());
    }
    let backup = database.with_extension(format!("db.before-{version}"));
    std::fs::copy(&database, &backup)?;
    println!("Local state backed up to {}", backup.display());
    Ok(())
}

/// Apunta la tarea de inicio de sesión a un ejecutable y registra la actualización como pendiente.
///
/// # Errors
/// Si el Programador de tareas rechaza el cambio.
#[cfg(windows)]
pub fn activate(
    store: &Store,
    executable: &Path,
    version: &Version,
    root_override: Option<&Path>,
) -> anyhow::Result<()> {
    let previous = std::env::current_exe()?;
    crate::install::point_task_to(executable, root_override)?;
    mark_pending(
        store,
        &Pending {
            version: version.to_string(),
            previous_executable: previous,
            previous_version: crate::cloud::AGENT_VERSION.to_owned(),
            attempts: 0,
            started_at: Utc::now(),
        },
    )?;
    Ok(())
}

/// Vuelve a la versión anterior: la tarea apunta otra vez al ejecutable previo.
///
/// # Errors
/// Si el ejecutable anterior ya no existe o el Programador de tareas falla.
#[cfg(windows)]
pub fn roll_back(store: &Store, pending: &Pending, root_override: Option<&Path>) -> anyhow::Result<()> {
    if !pending.previous_executable.is_file() {
        // Sin binario al que volver, lo honesto es dejar constancia y no tocar la tarea.
        confirm(store)?;
        anyhow::bail!("cannot roll back: {} no longer exists", pending.previous_executable.display());
    }
    crate::install::point_task_to(&pending.previous_executable, root_override)?;
    confirm(store)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for index in 0..4 {
                if index <= chunk.len() {
                    out.push(ALPHABET[((n >> (18 - index * 6)) & 0x3F) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn release_key() -> (SigningKey, Vec<PublicKey>) {
        let signing = SigningKey::from_bytes(&[5u8; 32]);
        let public =
            PublicKey::from_base64("release", &encode_base64(signing.verifying_key().as_bytes())).expect("public key");
        (signing, vec![public])
    }

    fn manifest(version: &str, body: &[u8]) -> Manifest {
        Manifest {
            schema: MANIFEST_SCHEMA.to_owned(),
            version: version.to_owned(),
            channel: "stable".to_owned(),
            url: "https://updates.example.com/jarvis-agent.exe".to_owned(),
            sha256: format!("{:x}", Sha256::digest(body)),
            size: body.len() as u64,
            released_at: Utc::now(),
            min_upgradable_from: None,
            signature: Signature { alg: "Ed25519".to_owned(), key_id: "release".to_owned(), value: String::new() },
        }
    }

    fn signed(mut manifest: Manifest, key: &SigningKey) -> Manifest {
        manifest.signature.value = encode_base64(&key.sign(manifest.signed_message().as_bytes()).to_bytes());
        manifest
    }

    #[test]
    fn versions_compare_including_prereleases() {
        let v = |text: &str| Version::parse(text).expect("version");
        assert!(v("1.2.0").is_newer_than(&v("1.1.9")));
        assert!(v("2.0.0").is_newer_than(&v("1.9.9")));
        assert!(!v("1.0.0").is_newer_than(&v("1.0.0")));
        assert!(!v("0.9.0").is_newer_than(&v("1.0.0")));
        assert!(v("1.0.0").is_newer_than(&v("1.0.0-rc.1")), "la final es más nueva que su prerelease");
        assert!(!v("1.0.0-rc.1").is_newer_than(&v("1.0.0")));
        assert!(v("1.0.0-rc.2").is_newer_than(&v("1.0.0-rc.1")));
        for invalid in ["1.0", "1.0.0.0", "uno.cero.cero", "1.0.0-", ""] {
            assert!(Version::parse(invalid).is_err(), "debe rechazarse: {invalid}");
        }
    }

    #[test]
    fn a_correctly_signed_manifest_is_accepted() {
        let (key, keys) = release_key();
        let plan = check(&signed(manifest("2.0.0", b"binario"), &key), "1.0.0", "stable", &keys).expect("plan");
        assert_eq!(plan.version.to_string(), "2.0.0");
        assert_eq!(plan.size, 7);
    }

    #[test]
    fn an_unsigned_or_tampered_manifest_is_refused() {
        let (key, keys) = release_key();
        let original = signed(manifest("2.0.0", b"binario"), &key);

        // Sin firma.
        let mut unsigned = original.clone();
        unsigned.signature.value = String::new();
        assert!(check(&unsigned, "1.0.0", "stable", &keys).is_err());

        // Firmado por otra clave.
        let impostor = SigningKey::from_bytes(&[6u8; 32]);
        assert!(check(&signed(manifest("2.0.0", b"binario"), &impostor), "1.0.0", "stable", &keys).is_err());

        // Cada campo firmado, alterado por separado.
        for tamper in [
            |m: &mut Manifest| m.url = "https://evil.example.com/agent.exe".to_owned(),
            |m: &mut Manifest| m.sha256 = "0".repeat(64),
            |m: &mut Manifest| m.size += 1,
            |m: &mut Manifest| m.version = "3.0.0".to_owned(),
            |m: &mut Manifest| m.channel = "beta".to_owned(),
        ] {
            let mut tampered = original.clone();
            tamper(&mut tampered);
            assert!(check(&tampered, "1.0.0", "stable", &keys).is_err(), "alterar un campo firmado debe invalidar");
        }
    }

    #[test]
    fn downgrades_and_other_channels_are_refused() {
        let (key, keys) = release_key();
        assert!(check(&signed(manifest("1.0.0", b"x"), &key), "1.0.0", "stable", &keys).is_err(), "misma versión");
        assert!(check(&signed(manifest("0.9.0", b"x"), &key), "1.0.0", "stable", &keys).is_err(), "anterior");
        assert!(check(&signed(manifest("2.0.0", b"x"), &key), "1.0.0", "beta", &keys).is_err(), "otro canal");

        let mut requires = manifest("3.0.0", b"x");
        requires.min_upgradable_from = Some("2.0.0".to_owned());
        assert!(check(&signed(requires, &key), "1.0.0", "stable", &keys).is_err(), "salto no permitido");
    }

    #[test]
    fn implausible_manifests_are_refused_before_downloading() {
        let (key, keys) = release_key();
        let cases: [fn(&mut Manifest); 4] = [
            |m| m.size = 0,
            |m| m.size = 200 * 1024 * 1024,
            |m| m.sha256 = "abc".to_owned(),
            |m| m.url = "http://updates.example.com/agent.exe".to_owned(),
        ];
        for tamper in cases {
            let mut candidate = manifest("2.0.0", b"binario");
            tamper(&mut candidate);
            assert!(check(&signed(candidate, &key), "1.0.0", "stable", &keys).is_err());
        }
    }

    #[test]
    fn the_downloaded_binary_must_match_the_manifest() {
        let (key, keys) = release_key();
        let body = b"contenido del binario";
        let plan = check(&signed(manifest("2.0.0", body), &key), "1.0.0", "stable", &keys).expect("plan");

        check_binary(&plan, body).expect("coincide");
        assert!(check_binary(&plan, b"contenido alterado!!!").is_err(), "otro contenido del mismo tamaño");
        assert!(check_binary(&plan, b"corto").is_err(), "otro tamaño");
    }

    #[test]
    fn staging_writes_the_binary_under_its_version() {
        let (key, keys) = release_key();
        let body = b"binario";
        let plan = check(&signed(manifest("2.1.0", body), &key), "1.0.0", "stable", &keys).expect("plan");
        let dir = tempfile::tempdir().expect("tempdir");

        let path = stage(dir.path(), &plan, body).expect("stage");
        assert!(path.ends_with(r"versions\2.1.0\jarvis-agent.exe"), "{}", path.display());
        assert_eq!(std::fs::read(&path).expect("read"), body);
        assert!(!dir.path().join("versions/2.1.0/jarvis-agent.exe.part").exists(), "no queda el temporal");
    }

    #[test]
    fn the_pending_update_survives_restarts_and_gives_up_after_three_attempts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("agent.db")).expect("store");
        let record = Pending {
            version: "2.0.0".to_owned(),
            previous_executable: PathBuf::from(r"C:\Programs\Jarvis\versions\1.0.0\jarvis-agent.exe"),
            previous_version: "1.0.0".to_owned(),
            attempts: 0,
            started_at: Utc::now(),
        };
        mark_pending(&store, &record).expect("mark");

        let mut current = pending(&store).expect("read").expect("some");
        assert_eq!(current.version, "2.0.0");
        for expected in 1..=MAX_UPDATE_ATTEMPTS {
            current = record_attempt(&store, current).expect("attempt");
            assert_eq!(current.attempts, expected);
        }
        assert!(should_roll_back(&current), "tras 3 intentos toca volver atrás");

        confirm(&store).expect("confirm");
        assert!(pending(&store).expect("read").is_none(), "confirmada deja de estar pendiente");
    }

    #[test]
    fn a_build_without_an_embedded_key_cannot_update() {
        if EMBEDDED_PUBLIC_KEY.is_none() {
            let error = release_keys().expect_err("sin clave no hay actualización");
            assert!(error.to_string().contains("no update key"), "{error}");
        }
    }
}
