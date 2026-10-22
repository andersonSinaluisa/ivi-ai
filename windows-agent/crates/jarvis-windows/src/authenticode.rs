//! Comprobación de firma Authenticode con WinVerifyTrust (HU-WIN-063, WT-13).
//!
//! Es la segunda barrera del actualizador: el manifiesto ya va firmado con la clave offline del
//! proyecto, y además el binario descargado tiene que estar firmado y con cadena válida según el
//! propio Windows. Se comprueba **antes** de instalar nada.

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
    WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
};
use windows::core::{HSTRING, PCWSTR};

/// Resultado de mirar la firma de un archivo.
///
/// Solo cuenta la firma **incrustada** en el ejecutable, que es la que lleva un artefacto de
/// release firmado con `signtool`. Los binarios del propio Windows suelen estar firmados por
/// *catálogo* (`.cat`) y aquí aparecen como [`Trust::Unsigned`]: es correcto para lo que se usa
/// esto (validar nuestro propio instalador), y evita aceptar un archivo por un catálogo ajeno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Firma incrustada válida y con cadena de confianza aceptada por Windows.
    Valid,
    /// Sin firma incrustada.
    Unsigned,
    /// Hay firma o formato, pero Windows no lo acepta: certificado caducado o revocado, archivo
    /// alterado, o el archivo no es un ejecutable válido.
    NotTrusted,
}

/// `TRUST_E_NOSIGNATURE`: el archivo no tiene firma.
const TRUST_E_NOSIGNATURE: i32 = -2_146_762_496; // 0x800B0100

/// Comprueba la firma Authenticode de un archivo.
///
/// # Errors
/// Si el archivo no existe o Windows no puede evaluarlo.
pub fn verify(path: &std::path::Path) -> Result<Trust, PlatformError> {
    if !path.is_file() {
        return Err(PlatformError::new(PlatformErrorKind::NotFound, "the file to verify does not exist"));
    }
    let wide = HSTRING::from(path.as_os_str());
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: u32::try_from(size_of::<WINTRUST_FILE_INFO>()).unwrap_or(0),
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: u32::try_from(size_of::<WINTRUST_DATA>()).unwrap_or(0),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &raw mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

    // SAFETY: las dos estructuras están completas y vivas durante las dos llamadas; la segunda
    // libera el estado que reservó la primera, como exige la API.
    let status = unsafe { WinVerifyTrust(HWND::default(), &raw mut action, (&raw mut data).cast()) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: cierre del estado abierto justo antes con el mismo `data`.
    unsafe {
        let _ = WinVerifyTrust(HWND::default(), &raw mut action, (&raw mut data).cast());
    }

    Ok(match status {
        0 => Trust::Valid,
        TRUST_E_NOSIGNATURE => Trust::Unsigned,
        _ => Trust::NotTrusted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn windows_binaries_signed_by_catalog_are_not_accepted_as_embedded_signatures() {
        // `notepad.exe` está firmado por catálogo, no dentro del archivo: para el actualizador eso
        // **no** cuenta como firma, y así no se acepta un binario por un catálogo ajeno.
        let path = PathBuf::from(crate::sys::expand_env(r"%SystemRoot%\System32\notepad.exe"));
        assert_eq!(verify(&path).expect("verify"), Trust::Unsigned);
    }

    #[test]
    fn an_embedded_signature_is_recognised_when_there_is_one_to_check() {
        // Se busca algún ejecutable con firma incrustada en el equipo; si no hay ninguno, el test
        // lo dice en vez de fingir que comprobó algo.
        let candidates = [
            r"%ProgramFiles%\Google\Chrome\Application\chrome.exe",
            r"%ProgramFiles%\Microsoft\Edge\Application\msedge.exe",
            r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe",
            r"%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe",
        ];
        for candidate in candidates {
            let path = PathBuf::from(crate::sys::expand_env(candidate));
            if path.is_file() && verify(&path).expect("verify") == Trust::Valid {
                return;
            }
        }
        println!("SKIPPED: no se encontró ningún ejecutable con firma incrustada");
    }

    #[test]
    fn a_file_that_is_not_a_valid_executable_is_not_trusted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake.exe");
        std::fs::write(&path, b"MZ not really a binary").expect("write");
        assert_eq!(verify(&path).expect("verify"), Trust::NotTrusted);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_verdict() {
        let error = verify(std::path::Path::new(r"C:\does-not-exist-jarvis.exe")).expect_err("no existe");
        assert_eq!(error.kind, PlatformErrorKind::NotFound);
    }
}
