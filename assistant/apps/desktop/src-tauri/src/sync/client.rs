//! Cliente HTTPS del Cloud Core (sync-protocol.md §10): TLS, timeouts, token de device renovado una vez ante 401,
//! errores clasificados en reintentables o definitivos. Las conexiones las inicia siempre el escritorio.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use super::protocol::{
    AccessToken, ConflictDto, HealthResponse, LoginResponse, PullRequest, PullResponse, PushRequest, PushResponse, RegisteredDevice,
    ResolveRequest, SearchRequest, SearchResponse, PROTOCOL,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
    /// Sin conexión, DNS, TLS, timeout: se reintenta con backoff.
    #[error("Cloud Core no disponible: {0}")]
    Unreachable(String),
    /// 429, 502, 503, 504: se reintenta con backoff.
    #[error("Cloud Core ocupado ({0})")]
    Busy(u16),
    /// 401 tras renovar el token: device revocado, deshabilitado o credencial rotada.
    #[error("credencial del device rechazada: vuelve a emparejar este equipo")]
    Unauthorized,
    /// 403: falta un permiso de sincronización.
    #[error("sin permiso: {0}")]
    Forbidden(String),
    /// Otros 4xx: la petición es incorrecta y no se reintenta igual.
    #[error("el Cloud Core rechazó la petición ({status}): {code} {detail}")]
    Rejected { status: u16, code: String, detail: String },
    #[error("configuración: {0}")]
    Config(String),
}

impl CloudError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Unreachable(_) | Self::Busy(_))
    }
}

pub type CloudResult<T> = Result<T, CloudError>;

/// Origen de la credencial del device (Credential Manager en Windows).
pub trait DeviceSecretSource: Send + Sync {
    fn device_id(&self) -> Option<String>;
    fn secret(&self) -> Option<String>;
}

pub struct CloudClient {
    base: String,
    http: Client,
    token: Mutex<Option<(String, DateTime<Utc>)>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub request: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { connect: Duration::from_secs(5), request: Duration::from_secs(20) }
    }
}

/// Valida la URL: HTTPS obligatorio salvo hacia loopback (desarrollo).
///
/// # Errors
/// URL inválida o HTTP plano hacia un host remoto.
pub fn validate_base_url(url: &str) -> CloudResult<String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|e| CloudError::Config(format!("URL inválida: {e}")))?;
    let host = parsed.host_str().unwrap_or_default();
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1";
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(CloudError::Config("el Cloud Core requiere HTTPS (HTTP solo hacia localhost)".into())),
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

impl CloudClient {
    /// # Errors
    /// URL inválida o cliente HTTP imposible de construir.
    pub fn new(base_url: &str, timeouts: Timeouts) -> CloudResult<Self> {
        let base = validate_base_url(base_url)?;
        let http = Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.request)
            .user_agent(concat!("jarvis-desktop/", env!("CARGO_PKG_VERSION")))
            .https_only(!base.starts_with("http://"))
            .build()
            .map_err(|e| CloudError::Config(e.to_string()))?;
        Ok(Self { base, http, token: Mutex::new(None) })
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn send<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> CloudResult<T> {
        let response = request.send().await.map_err(|e| CloudError::Unreachable(e.without_url().to_string()))?;
        let status = response.status();
        if status.is_success() {
            return response.json::<T>().await.map_err(|e| CloudError::Unreachable(format!("respuesta inválida: {e}")));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let code = body.get("code").and_then(Value::as_str).unwrap_or_default().to_owned();
        let detail = body.get("detail").and_then(Value::as_str).unwrap_or_default().to_owned();
        Err(match status {
            StatusCode::UNAUTHORIZED => CloudError::Unauthorized,
            StatusCode::FORBIDDEN => CloudError::Forbidden(if code.is_empty() { detail } else { code }),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
                CloudError::Busy(status.as_u16())
            }
            s if s.is_server_error() => CloudError::Busy(s.as_u16()),
            s => CloudError::Rejected { status: s.as_u16(), code, detail },
        })
    }

    /// Salud y capacidades; también mide la latencia.
    ///
    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn health(&self) -> CloudResult<(HealthResponse, Duration)> {
        let started = std::time::Instant::now();
        let health: HealthResponse = self.send(self.http.get(self.url("/api/v1/health"))).await?;
        Ok((health, started.elapsed()))
    }

    // ------------------------------------------------------------------ emparejamiento (user)

    /// Emparejamiento: inicia sesión como owner (la contraseña no se guarda), registra este equipo como device y le
    /// concede los scopes de sincronización. Devuelve la credencial del device (se muestra una sola vez).
    ///
    /// # Errors
    /// Credenciales inválidas, nombre en uso, Cloud no disponible…
    pub async fn pair(&self, email: &str, password: &str, device_name: &str, scopes: &[&str]) -> CloudResult<RegisteredDevice> {
        let login: LoginResponse = self
            .send(self.http.post(self.url("/api/v1/auth/login")).json(&serde_json::json!({ "email": email, "password": password })))
            .await?;
        let registered: RegisteredDevice = self
            .send(
                self.http
                    .post(self.url("/api/v1/devices"))
                    .bearer_auth(&login.access_token)
                    .json(&serde_json::json!({ "name": device_name, "type": "Windows" })),
            )
            .await?;
        let _: Value = self
            .send(
                self.http
                    .put(self.url(&format!("/api/v1/sync/devices/{}", registered.device.id)))
                    .bearer_auth(&login.access_token)
                    .json(&serde_json::json!({ "scopes": scopes })),
            )
            .await?;
        Ok(registered)
    }

    // ------------------------------------------------------------------ device

    pub(crate) async fn token(&self, secrets: &dyn DeviceSecretSource, force: bool) -> CloudResult<String> {
        self.token_valid_for(secrets, force, chrono::Duration::seconds(30)).await
    }

    /// Token que seguirá siendo válido al menos `min_validity` (si no, se pide uno nuevo).
    pub(crate) async fn token_valid_for(
        &self,
        secrets: &dyn DeviceSecretSource,
        force: bool,
        min_validity: chrono::Duration,
    ) -> CloudResult<String> {
        let mut guard = self.token.lock().await;
        if !force {
            if let Some((token, expires)) = guard.as_ref() {
                if *expires - min_validity > Utc::now() {
                    return Ok(token.clone());
                }
            }
        }
        let (Some(device_id), Some(secret)) = (secrets.device_id(), secrets.secret()) else {
            return Err(CloudError::Config("este equipo no está emparejado".into()));
        };
        let issued: AccessToken = self
            .send(
                self.http.post(self.url("/api/v1/auth/device-token")).json(&serde_json::json!({ "deviceId": device_id, "secret": secret })),
            )
            .await
            .map_err(|e| match e {
                CloudError::Rejected { status: 401 | 403, .. } | CloudError::Forbidden(_) => CloudError::Unauthorized,
                other => other,
            })?;
        *guard = Some((issued.access_token.clone(), issued.access_token_expires_at));
        Ok(issued.access_token)
    }

    /// Petición como device: si responde 401, renueva el token una vez y reintenta.
    async fn device_call<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        secrets: &dyn DeviceSecretSource,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
        timeout: Option<Duration>,
    ) -> CloudResult<T> {
        for attempt in 0..2 {
            let token = self.token(secrets, attempt > 0).await?;
            let mut request = self.http.request(method.clone(), self.url(path)).bearer_auth(token);
            if let Some(body) = body {
                request = request.json(body);
            }
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            match self.send(request).await {
                Err(CloudError::Unauthorized) if attempt == 0 => {}
                other => return other,
            }
        }
        Err(CloudError::Unauthorized)
    }

    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn push(&self, secrets: &dyn DeviceSecretSource, request: &PushRequest) -> CloudResult<PushResponse> {
        self.device_call(secrets, reqwest::Method::POST, "/api/v1/sync/push", Some(request), None).await
    }

    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn pull(&self, secrets: &dyn DeviceSecretSource, since: i64, limit: u32, embedding_models: Vec<String>) -> CloudResult<PullResponse> {
        let request = PullRequest { protocol: PROTOCOL.into(), since, limit, entities: Vec::new(), embedding_models };
        self.device_call(secrets, reqwest::Method::POST, "/api/v1/sync/pull", Some(&request), None).await
    }

    /// Búsqueda en el Cloud durante una conversación: timeout corto, nunca bloquea la respuesta local.
    ///
    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn search(&self, secrets: &dyn DeviceSecretSource, request: &SearchRequest, timeout: Duration) -> CloudResult<SearchResponse> {
        self.device_call(secrets, reqwest::Method::POST, "/api/v1/memories/search", Some(request), Some(timeout)).await
    }

    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn conflicts(&self, secrets: &dyn DeviceSecretSource) -> CloudResult<Vec<ConflictDto>> {
        #[derive(serde::Deserialize)]
        struct Items {
            items: Vec<ConflictDto>,
        }
        let items: Items = self.device_call::<(), _>(secrets, reqwest::Method::GET, "/api/v1/sync/conflicts", None, None).await?;
        Ok(items.items)
    }

    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn resolve(&self, secrets: &dyn DeviceSecretSource, conflict_id: &str, request: &ResolveRequest) -> CloudResult<Value> {
        self.device_call(secrets, reqwest::Method::POST, &format!("/api/v1/sync/conflicts/{conflict_id}/resolve"), Some(request), None)
            .await
    }

    /// # Errors
    /// Errores de red o del Cloud.
    pub async fn status(&self, secrets: &dyn DeviceSecretSource) -> CloudResult<Value> {
        self.device_call::<(), _>(secrets, reqwest::Method::GET, "/api/v1/sync/status", None, None).await
    }

    /// Olvida el token cacheado (tras reemparejar).
    pub async fn reset_token(&self) {
        *self.token.lock().await = None;
    }
}

/// Backoff exponencial con jitter: 1 s, 2 s, 4 s… hasta 5 min.
#[must_use]
pub fn backoff(failures: u32) -> Duration {
    let base = 1u64 << failures.min(9);
    let capped = base.min(300);
    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]) % (capped / 4 + 1);
    Duration::from_secs(capped - capped / 8 + jitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_required_except_for_loopback() {
        assert!(validate_base_url("https://jarvis.example.com/").is_ok());
        assert_eq!(validate_base_url("http://127.0.0.1:8080/").expect("loopback"), "http://127.0.0.1:8080");
        assert!(validate_base_url("http://jarvis.example.com").is_err());
        assert!(validate_base_url("ftp://x").is_err());
        assert!(validate_base_url("no es url").is_err());
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(backoff(0) <= Duration::from_secs(2));
        assert!(backoff(3) >= Duration::from_secs(7));
        assert!(backoff(30) <= Duration::from_secs(375));
        assert!(backoff(30) >= Duration::from_secs(260));
    }
}
