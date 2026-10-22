//! Descarga HTTPS con WinHTTP, solo para el actualizador (HU-WIN-063).
//!
//! Se usa la pila del sistema en vez de un cliente propio: así la validación de certificados, las
//! CA y la configuración de proxy de la empresa son las de Windows (ADR-002). Reglas: **solo
//! `https`**, sin redirecciones a texto plano, tamaño acotado y sin cookies ni credenciales.

use std::ffi::c_void;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WinHttpCloseHandle,
    WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
};
use windows::core::{HSTRING, PCWSTR};

use crate::sys::last_error;

/// Handle de WinHTTP que se cierra al soltarse.
struct WinHttpHandle(*mut c_void);

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle obtenido de WinHTTP y cerrado una sola vez.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// `https://host[:port]/ruta` partido en sus piezas.
struct Url {
    host: String,
    port: u16,
    path: String,
}

fn parse_https(url: &str) -> Option<Url> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = rest.split_once('/').map_or((rest, String::from("/")), |(a, p)| (a, format!("/{p}")));
    if authority.contains('@') {
        return None; // Credenciales en la URL: nunca.
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (authority, INTERNET_DEFAULT_HTTPS_PORT),
    };
    if host.is_empty() {
        return None;
    }
    Some(Url { host: host.to_owned(), port, path })
}

/// Descarga el contenido de una URL `https`, como mucho `max_bytes`.
///
/// # Errors
/// `Other` con el motivo: URL no https, fallo de red o TLS, respuesta demasiado grande.
pub fn get(url: &str, max_bytes: usize) -> Result<Vec<u8>, PlatformError> {
    let parsed = parse_https(url)
        .ok_or_else(|| PlatformError::new(PlatformErrorKind::Other, "only plain https URLs are accepted"))?;

    let agent = HSTRING::from(format!("JarvisAgent/{}", env!("CARGO_PKG_VERSION")));
    let host = HSTRING::from(parsed.host.as_str());
    let path = HSTRING::from(parsed.path.as_str());

    // SAFETY: todos los handles se cierran al soltar los `WinHttpHandle`; las cadenas viven hasta
    // el final de la función y los buffers son propios.
    unsafe {
        let session =
            WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let session = WinHttpHandle(session);

        // Una redirección nunca puede degradar a http.
        let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(std::slice::from_raw_parts((&raw const policy).cast::<u8>(), size_of::<u32>())),
        )
        .map_err(|_| last_error("WinHttpSetOption(REDIRECT_POLICY)"))?;

        let connection = WinHttpConnect(session.0, PCWSTR(host.as_ptr()), parsed.port, 0);
        if connection.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let connection = WinHttpHandle(connection);

        let request = WinHttpOpenRequest(
            connection.0,
            PCWSTR(HSTRING::from("GET").as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        let request = WinHttpHandle(request);

        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|_| last_error("WinHttpSendRequest"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| last_error("WinHttpReceiveResponse"))?;

        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &raw mut available)
                .map_err(|_| last_error("WinHttpQueryDataAvailable"))?;
            if available == 0 {
                break;
            }
            let chunk = available as usize;
            if body.len() + chunk > max_bytes {
                return Err(PlatformError::new(PlatformErrorKind::Other, "the response exceeds the allowed size"));
            }
            let mut buffer = vec![0u8; chunk];
            let mut read = 0u32;
            WinHttpReadData(request.0, buffer.as_mut_ptr().cast::<c_void>(), available, &raw mut read)
                .map_err(|_| last_error("WinHttpReadData"))?;
            buffer.truncate(read as usize);
            body.extend_from_slice(&buffer);
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_are_accepted() {
        for url in [
            "http://example.com/manifest.json",
            "ftp://example.com/x",
            "https://user:pass@example.com/x",
            "https:///x",
            "example.com/x",
        ] {
            assert!(parse_https(url).is_none(), "debe rechazarse: {url}");
        }
    }

    #[test]
    fn splits_host_port_and_path() {
        let url = parse_https("https://updates.example.com:8443/jarvis/manifest.json").expect("url");
        assert_eq!(
            (url.host.as_str(), url.port, url.path.as_str()),
            ("updates.example.com", 8443, "/jarvis/manifest.json")
        );

        let url = parse_https("https://updates.example.com").expect("url");
        assert_eq!((url.host.as_str(), url.port, url.path.as_str()), ("updates.example.com", 443, "/"));
    }

    #[test]
    fn a_plain_http_url_never_reaches_the_network() {
        let error = get("http://example.com/manifest.json", 1024).expect_err("http rechazado");
        assert!(error.message.contains("https"), "{error}");
    }
}
