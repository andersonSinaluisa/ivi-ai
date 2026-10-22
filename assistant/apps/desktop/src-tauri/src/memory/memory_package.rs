//! Formato del archivo `.jarvismem`: un tar con la carpeta `jarvis-memory/`, cifrado.
//!
//! ```text
//! "JARVISMEM1" | sal (16) | nonce (24) | m_cost KiB (u32 LE) | t_cost (u32 LE) | p_cost (u32 LE) | cifrado
//! ```
//! Clave: Argon2id(contraseña, sal). Cifrado: XChaCha20-Poly1305 con la cabecera como datos
//! asociados, así que alterar cualquier byte (también los parámetros) hace fallar el descifrado.

use std::path::{Component, Path};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::{MemoryError, MemoryResult};

pub const MAGIC: &[u8; 10] = b"JARVISMEM1";
pub const ROOT: &str = "jarvis-memory";
const HEADER_LEN: usize = 10 + 16 + 24 + 12;
/// Tamaño máximo de un paquete (descomprimido o cifrado).
pub const MAX_PACKAGE_BYTES: usize = 512 * 1024 * 1024;
pub const MIN_PASSPHRASE: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    /// 64 MiB y 3 pasadas: lento para un atacante, ~0,5 s para el usuario.
    fn default() -> Self {
        Self { m_cost_kib: 64 * 1024, t_cost: 3, p_cost: 1 }
    }
}

impl KdfParams {
    /// Parámetros baratos para pruebas.
    #[must_use]
    pub const fn fast() -> Self {
        Self { m_cost_kib: 1024, t_cost: 1, p_cost: 1 }
    }
}

fn err(msg: impl Into<String>) -> MemoryError {
    MemoryError::Package(msg.into())
}

fn derive_key(passphrase: &str, salt: &[u8], kdf: KdfParams) -> MemoryResult<[u8; 32]> {
    let params = Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(32)).map_err(|e| err(e.to_string()))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| err(e.to_string()))?;
    Ok(key)
}

/// Cifra el contenido con la contraseña.
///
/// # Errors
/// Contraseña demasiado corta o fallo criptográfico.
pub fn seal(plaintext: &[u8], passphrase: &str, kdf: KdfParams) -> MemoryResult<Vec<u8>> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(err(format!("la contraseña debe tener al menos {MIN_PASSPHRASE} caracteres")));
    }
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    for v in [kdf.m_cost_kib, kdf.t_cost, kdf.p_cost] {
        header.extend_from_slice(&v.to_le_bytes());
    }
    let key = derive_key(passphrase, &salt, kdf)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher.encrypt(&nonce, Payload { msg: plaintext, aad: &header }).map_err(|_| err("no se pudo cifrar"))?;
    let mut out = header;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Descifra un paquete.
///
/// # Errors
/// Formato desconocido, parámetros abusivos, contraseña incorrecta o archivo alterado.
pub fn open(data: &[u8], passphrase: &str) -> MemoryResult<Vec<u8>> {
    if data.len() < HEADER_LEN + 16 || &data[..10] != MAGIC {
        return Err(err("no es un paquete de memoria de Jarvis (.jarvismem)"));
    }
    if data.len() > MAX_PACKAGE_BYTES {
        return Err(err("el paquete supera el tamaño máximo"));
    }
    let header = &data[..HEADER_LEN];
    let salt = &header[10..26];
    let nonce = XNonce::from_slice(&header[26..50]);
    let u = |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let kdf = KdfParams { m_cost_kib: u(50), t_cost: u(54), p_cost: u(58) };
    // Un archivo manipulado no debe poder agotar la memoria o la CPU al derivar la clave.
    if kdf.m_cost_kib > 1024 * 1024 || kdf.t_cost > 10 || kdf.p_cost > 8 || kdf.m_cost_kib < 8 {
        return Err(err("parámetros de cifrado fuera de rango"));
    }
    let key = derive_key(passphrase, salt, kdf)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(nonce, Payload { msg: &data[HEADER_LEN..], aad: header })
        .map_err(|_| err("contraseña incorrecta o archivo dañado"))
}

/// Empaqueta archivos (rutas relativas dentro de `jarvis-memory/`) en un tar.
///
/// # Errors
/// Errores de E/S del tar.
pub fn tar(files: &[(String, Vec<u8>)], dirs: &[&str]) -> MemoryResult<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    let mtime = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    for d in dirs {
        let mut h = tar::Header::new_ustar();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_size(0);
        h.set_mode(0o755);
        h.set_mtime(mtime);
        builder.append_data(&mut h, format!("{ROOT}/{d}/"), std::io::empty()).map_err(|e| err(e.to_string()))?;
    }
    for (name, data) in files {
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(mtime);
        builder.append_data(&mut h, format!("{ROOT}/{name}"), data.as_slice()).map_err(|e| err(e.to_string()))?;
    }
    builder.into_inner().map_err(|e| err(e.to_string()))
}

/// Extrae los archivos validando las rutas (nada fuera de `jarvis-memory/`, sin `..`).
///
/// # Errors
/// Rutas peligrosas o tamaños excesivos.
pub fn untar(data: &[u8]) -> MemoryResult<Vec<(String, Vec<u8>)>> {
    let mut archive = tar::Archive::new(data);
    let mut out = Vec::new();
    let mut total = 0usize;
    for entry in archive.entries().map_err(|e| err(e.to_string()))? {
        let mut entry = entry.map_err(|e| err(e.to_string()))?;
        let path = entry.path().map_err(|e| err(e.to_string()))?.into_owned();
        if !path.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(err(format!("ruta no permitida en el paquete: {}", path.display())));
        }
        let rel = path.strip_prefix(ROOT).map_err(|_| err(format!("archivo fuera de {ROOT}/: {}", path.display())))?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(err("el paquete contiene enlaces u otros tipos de archivo"));
        }
        let size = usize::try_from(entry.size()).unwrap_or(usize::MAX);
        total = total.saturating_add(size);
        if total > MAX_PACKAGE_BYTES {
            return Err(err("el contenido del paquete es demasiado grande"));
        }
        let mut buf = Vec::with_capacity(size);
        std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| err(e.to_string()))?;
        out.push((rel.to_string_lossy().replace('\\', "/"), buf));
    }
    Ok(out)
}

/// Solo `.jarvismem`.
///
/// # Errors
/// Extensión incorrecta.
pub fn check_extension(path: &Path) -> MemoryResult<()> {
    if path.extension().and_then(|e| e.to_str()) == Some("jarvismem") {
        Ok(())
    } else {
        Err(err("el archivo debe tener la extensión .jarvismem"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_round_trip() {
        let sealed = seal(b"hola memoria", "una clave larga", KdfParams::fast()).expect("seal");
        assert_eq!(&sealed[..10], MAGIC);
        assert_eq!(open(&sealed, "una clave larga").expect("open"), b"hola memoria");
    }

    #[test]
    fn wrong_passphrase_tampering_and_short_passwords_fail() {
        let mut sealed = seal(b"datos", "una clave larga", KdfParams::fast()).expect("seal");
        assert!(open(&sealed, "otra clave larga").is_err());
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(open(&sealed, "una clave larga").is_err(), "integridad");
        assert!(seal(b"x", "corta", KdfParams::fast()).is_err());
        assert!(open(b"no soy un paquete", "una clave larga").is_err());
    }

    #[test]
    fn hostile_kdf_parameters_are_rejected() {
        let mut sealed = seal(b"datos", "una clave larga", KdfParams::fast()).expect("seal");
        sealed[50..54].copy_from_slice(&(u32::MAX).to_le_bytes());
        let e = open(&sealed, "una clave larga").expect_err("rango");
        assert!(e.to_string().contains("fuera de rango"));
    }

    #[test]
    fn tar_round_trip_and_path_traversal_guard() {
        let t = tar(&[("manifest.json".into(), b"{}".to_vec()), ("skills/a.json".into(), b"[]".to_vec())], &["skills", "attachments"]).expect("tar");
        let files = untar(&t).expect("untar");
        assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["manifest.json", "skills/a.json"]);

        // Un tar con "../evil" no se acepta.
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o644);
        let name = b"jarvis-memory/../evil";
        h.as_gnu_mut().expect("gnu").name[..name.len()].copy_from_slice(name);
        h.set_cksum();
        b.append(&h, &b"x"[..]).expect("append");
        let evil = b.into_inner().expect("tar");
        assert!(untar(&evil).is_err());
    }
}
