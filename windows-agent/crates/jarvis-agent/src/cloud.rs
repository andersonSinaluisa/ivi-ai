//! Cloud Link: cliente MQTT del device (docs/02-cloud-integration).
//!
//! Implementa §5 del protocolo del Cloud: clean start, LWT con `sessionId`, suscripción a `down/#`,
//! `presence.hello` → `presence.hello.ack`, heartbeats y reconexión con backoff exponencial.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::Utc;
use jarvis_protocol::codes::message_types as mt;
use jarvis_protocol::messages::{PresenceGoodbye, PresenceHeartbeat, PresenceHello, PresenceHelloAck, PresenceOffline};
use jarvis_protocol::{Channel, Envelope, MAX_MESSAGE_BYTES, Topics};
use rumqttc::{AsyncClient, ConnectReturnCode, Event, LastWill, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use rustls::ClientConfig;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::BrokerUrl;
use crate::health::{Health, Severity, codes};
use crate::pipeline::{Agent, Publisher};

/// Versión del protocolo que declara el agente (1.1 añade `sessionState`, C11).
pub const PROTOCOL_VERSION: &str = "1.1";
pub const AGENT_VERSION: &str = concat!("jarvis-windows-agent/", env!("CARGO_PKG_VERSION"));

/// Backoff tras un rechazo de autenticación (credencial revocada o rotada).
const AUTH_BACKOFF: Duration = Duration::from_secs(300);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const HELLO_ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Mensajes entrantes por segundo que se aceptan; el exceso se descarta (el Cloud reintentará los
/// dispatches tras su ACK timeout). Evita que un flood consuma la cola y el disco.
const MAX_INBOUND_PER_SECOND: u32 = 50;

/// TLS 1.2+ validando cadena y hostname contra el almacén de certificados de Windows.
///
/// El parametro ca_file solo se usa en entornos de desarrollo con una CA propia.
///
/// # Errors
/// Si el PEM no se puede leer o el proveedor criptográfico falla.
pub fn tls_client_config(ca_file: Option<&std::path::Path>) -> anyhow::Result<ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(ca_file) = ca_file else {
        use rustls_platform_verifier::ConfigVerifierExt;
        return ClientConfig::with_platform_verifier().map_err(Into::into);
    };
    let pem = std::fs::read(ca_file)?;
    let mut roots = rustls::RootCertStore::empty();
    for certificate in rustls_pemfile::certs(&mut pem.as_slice()) {
        roots.add(certificate?)?;
    }
    Ok(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

pub struct CloudLink {
    device_id: Uuid,
    topics: Topics,
    broker: BrokerUrl,
    ca_file: Option<std::path::PathBuf>,
    keepalive: Duration,
    max_inflight: u16,
    health: Arc<Health>,
    state: Mutex<LinkState>,
}

/// Contador por segundo para el límite de tasa.
#[derive(Debug)]
struct RateLimiter {
    window: std::time::Instant,
    count: u32,
    dropped: u32,
    last_warning: Option<std::time::Instant>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self { window: std::time::Instant::now(), count: 0, dropped: 0, last_warning: None }
    }
}

impl RateLimiter {
    /// `true` si el mensaje se acepta.
    fn allow(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        self.count += 1;
        if self.count <= MAX_INBOUND_PER_SECOND {
            return true;
        }
        self.dropped += 1;
        let warn = self.last_warning.is_none_or(|last| now.duration_since(last) >= Duration::from_secs(60));
        if warn {
            self.last_warning = Some(now);
            tracing::warn!(dropped = self.dropped, "inbound rate limit exceeded; extra messages discarded");
            self.dropped = 0;
        }
        false
    }
}

#[derive(Debug, Default)]
struct LinkState {
    client: Option<AsyncClient>,
    session_id: String,
    ready: bool,
    heartbeat_interval: u64,
    rate: RateLimiter,
}

#[async_trait::async_trait]
impl Publisher for CloudLink {
    async fn publish(&self, channel: Channel, payload: Vec<u8>) -> bool {
        if payload.len() > MAX_MESSAGE_BYTES {
            tracing::error!(size = payload.len(), "outgoing message exceeds the protocol limit");
            return false;
        }
        let client = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if !state.ready {
                return false;
            }
            state.client.clone()
        };
        let Some(client) = client else { return false };
        match client.publish(self.topics.up(channel), QoS::AtLeastOnce, false, payload).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "publish failed; the message stays in the outbox");
                false
            }
        }
    }
}

/// Credencial del device (se lee de Credential Manager en cada intento de conexión).
pub trait SecretSource: Send + Sync {
    fn secret(&self) -> Option<String>;
}

impl CloudLink {
    #[must_use]
    pub fn new(
        device_id: Uuid,
        broker: BrokerUrl,
        ca_file: Option<std::path::PathBuf>,
        keepalive_seconds: u16,
        max_inflight: u16,
        health: Arc<Health>,
    ) -> Arc<Self> {
        Arc::new(Self {
            device_id,
            topics: Topics::new(device_id),
            broker,
            ca_file,
            keepalive: Duration::from_secs(u64::from(keepalive_seconds)),
            max_inflight,
            health,
            state: Mutex::new(LinkState::default()),
        })
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).ready
    }

    fn session_id(&self) -> String {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).session_id.clone()
    }

    fn heartbeat_interval(&self) -> Duration {
        let seconds = self.state.lock().unwrap_or_else(PoisonError::into_inner).heartbeat_interval;
        Duration::from_secs(seconds.clamp(5, 300))
    }

    /// Bucle principal: conectar, saludar, atender mensajes y reconectar con backoff.
    pub async fn run(
        self: Arc<Self>,
        agent: Arc<Agent>,
        secrets: Arc<dyn SecretSource>,
        reconnect: Arc<tokio::sync::Notify>,
        cancel: CancellationToken,
    ) {
        let mut backoff = Duration::from_secs(1);
        while !cancel.is_cancelled() {
            let Some(secret) = secrets.secret() else {
                self.health.raise(codes::NOT_PROVISIONED, Severity::Error, "run `jarvis-agent provision`");
                tokio::select! {
                    () = cancel.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_secs(30)) => continue,
                    () = reconnect.notified() => continue,
                }
            };
            self.health.clear(codes::NOT_PROVISIONED);

            match self.session(&agent, secret, &reconnect, &cancel).await {
                SessionEnd::AuthRejected => {
                    self.health.set_auth_rejected(true);
                    self.health.raise(
                        codes::AUTH_REJECTED,
                        Severity::Error,
                        "the broker rejected the credential; re-provision the device",
                    );
                    tracing::warn!("authentication rejected: the credential was revoked or rotated");
                    // La espera larga se corta si alguien reprovisiona: `jarvis-agent provision`
                    // pide reconexion, y el owner no deberia esperar cinco minutos por ello.
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        () = tokio::time::sleep(AUTH_BACKOFF) => {},
                        () = reconnect.notified() => tracing::info!("reconnect requested: retrying with the stored credential"),
                    }
                    continue;
                }
                SessionEnd::Cancelled => break,
                SessionEnd::ReconnectRequested => {
                    tracing::info!("immediate reconnect requested");
                    backoff = Duration::from_secs(1);
                    continue;
                }
                SessionEnd::Disconnected(reason) => {
                    tracing::warn!(reason, "connection lost");
                    self.health.set_connected(false, Some(reason));
                    self.health.raise(codes::CLOUD_UNREACHABLE, Severity::Warn, "reconnecting");
                }
            }

            // Backoff exponencial 1 s → 60 s con jitter ±20 %.
            let jitter = 0.8 + rand::random::<f64>() * 0.4;
            let wait = backoff.mul_f64(jitter).min(MAX_BACKOFF);
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(wait) => {},
                () = reconnect.notified() => backoff = Duration::from_secs(1),
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
        self.mark_disconnected();
    }

    fn mark_disconnected(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ready = false;
        state.client = None;
    }

    async fn session(
        &self,
        agent: &Arc<Agent>,
        secret: String,
        reconnect: &Arc<tokio::sync::Notify>,
        cancel: &CancellationToken,
    ) -> SessionEnd {
        let session_id = format!("s-{:016x}", rand::random::<u64>());
        let options = match self.options(&secret, &session_id) {
            Ok(options) => options,
            Err(error) => {
                tracing::error!(%error, "invalid MQTT configuration");
                return SessionEnd::Disconnected("invalid configuration".to_owned());
            }
        };
        let (client, mut eventloop) = AsyncClient::new(options, 64);
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.client = Some(client.clone());
            state.session_id = session_id.clone();
            state.ready = false;
            state.heartbeat_interval = 30;
        }

        let mut hello_deadline: Option<tokio::time::Instant> = None;
        loop {
            let event = tokio::select! {
                () = cancel.cancelled() => {
                    self.goodbye(&client, "shutdown").await;
                    return SessionEnd::Cancelled;
                }
                () = reconnect.notified() => {
                    // Reanudación tras suspender o credencial nueva: no se espera al backoff.
                    let _ = client.disconnect().await;
                    self.mark_disconnected();
                    return SessionEnd::ReconnectRequested;
                }
                event = eventloop.poll() => event,
            };
            match event {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => match ack.code {
                    ConnectReturnCode::Success => {
                        tracing::info!(broker = %self.broker.host, "connected");
                        self.health.set_connected(true, None);
                        self.health.clear(codes::CLOUD_UNREACHABLE);
                        if let Err(error) = client.subscribe(self.topics.subscription(), QoS::AtLeastOnce).await {
                            return SessionEnd::Disconnected(format!("subscribe failed: {error}"));
                        }
                        self.send_hello(&client, agent, &session_id).await;
                        hello_deadline = Some(tokio::time::Instant::now() + HELLO_ACK_TIMEOUT);
                    }
                    ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized => {
                        return SessionEnd::AuthRejected;
                    }
                    other => return SessionEnd::Disconnected(format!("CONNACK {other:?}")),
                },
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    self.on_message(agent, &publish.topic, &publish.payload, &session_id).await;
                    if self.is_ready() {
                        hello_deadline = None;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    // rumqttc entrega un CONNACK negativo como *error*, no como paquete: sin esto
                    // una credencial revocada se veria como "sin red" y nunca como AUTH_REJECTED.
                    if let rumqttc::ConnectionError::ConnectionRefused(code) = &error
                        && matches!(code, ConnectReturnCode::NotAuthorized | ConnectReturnCode::BadUserNamePassword)
                    {
                        return SessionEnd::AuthRejected;
                    }
                    return SessionEnd::Disconnected(error.to_string());
                }
            }

            if hello_deadline.is_some_and(|deadline| tokio::time::Instant::now() > deadline) {
                return SessionEnd::Disconnected("no presence.hello.ack within 10s".to_owned());
            }
        }
    }

    fn options(&self, secret: &str, session_id: &str) -> anyhow::Result<MqttOptions> {
        let id = self.device_id.hyphenated().to_string();
        let mut options = MqttOptions::new(id.clone(), &self.broker.host, self.broker.port);
        options.set_credentials(id, secret.to_owned());
        options.set_keep_alive(self.keepalive);
        // Clean start: el Cloud redespacha desde PostgreSQL; nada viejo encolado en el broker.
        options.set_clean_session(true);
        options.set_max_packet_size(MAX_MESSAGE_BYTES, MAX_MESSAGE_BYTES);
        options.set_inflight(self.max_inflight);

        let will = Envelope::create(
            mt::PRESENCE_OFFLINE,
            self.device_id,
            &PresenceOffline { session_id: session_id.to_owned() },
            None,
            None,
        )?;
        options.set_last_will(LastWill::new(
            self.topics.up(Channel::Presence),
            will.to_bytes()?,
            QoS::AtLeastOnce,
            false,
        ));

        if self.broker.tls {
            options.set_transport(Transport::Tls(TlsConfiguration::Rustls(Arc::new(self.tls_config()?))));
        }
        Ok(options)
    }

    fn tls_config(&self) -> anyhow::Result<ClientConfig> {
        tls_client_config(self.ca_file.as_deref())
    }

    async fn send_hello(&self, client: &AsyncClient, agent: &Arc<Agent>, session_id: &str) {
        let hello = PresenceHello {
            session_id: session_id.to_owned(),
            protocol_version: PROTOCOL_VERSION.to_owned(),
            software_version: AGENT_VERSION.to_owned(),
            platform: "windows".to_owned(),
            capabilities: vec!["agent".to_owned(), "windows-control".to_owned()],
            supported_task_types: agent.registry.supported_task_types(),
            open_task_ids: agent.open_task_ids(),
            busy: agent.busy(),
            session_state: Some(self.session_state()),
        };
        self.publish_presence(client, mt::PRESENCE_HELLO, &hello).await;
    }

    fn session_state(&self) -> String {
        // El estado real lo actualiza el bucle de sesión del agente; aquí se lee del health.
        self.health
            .snapshot(
                crate::health::DbSection { healthy: true, size_bytes: 0, outbox_pending: 0, unsettled: 0 },
                crate::health::ToolsSection { healthy: true, enabled: 0, running: 0, awaiting_approval: 0, stuck: 0 },
                AGENT_VERSION,
            )
            .session
            .state
    }

    async fn publish_presence<T: serde::Serialize>(&self, client: &AsyncClient, kind: &str, data: &T) {
        let Ok(envelope) = Envelope::create(kind, self.device_id, data, None, None) else { return };
        let Ok(payload) = envelope.to_bytes() else { return };
        let qos = if kind == mt::PRESENCE_HEARTBEAT { QoS::AtMostOnce } else { QoS::AtLeastOnce };
        if let Err(error) = client.publish(self.topics.up(Channel::Presence), qos, false, payload).await {
            tracing::warn!(%error, kind, "cannot publish presence");
        }
    }

    async fn on_message(&self, agent: &Arc<Agent>, topic: &str, payload: &[u8], session_id: &str) {
        if !self.state.lock().unwrap_or_else(PoisonError::into_inner).rate.allow() {
            return;
        }
        let Some(channel) = self.topics.parse_own_down(topic) else {
            tracing::warn!(topic, "message on an unexpected topic discarded");
            return;
        };
        let envelope = match Envelope::parse(payload) {
            Ok(envelope) => envelope,
            Err(error) => {
                tracing::warn!(%error, "invalid envelope discarded");
                return;
            }
        };
        if envelope.device_id != self.device_id {
            tracing::warn!("envelope deviceId does not match this device; discarded");
            return;
        }

        if channel == Channel::Control && envelope.kind == mt::PRESENCE_HELLO_ACK {
            match envelope.data_as::<PresenceHelloAck>() {
                Ok(ack) if ack.session_id == session_id => self.on_hello_ack(agent, &ack).await,
                Ok(_) => tracing::debug!("hello.ack from a previous session ignored"),
                Err(error) => tracing::warn!(%error, "invalid presence.hello.ack"),
            }
            return;
        }
        agent.handle_envelope(channel, envelope).await;
    }

    async fn on_hello_ack(&self, agent: &Arc<Agent>, ack: &PresenceHelloAck) {
        let offset = (ack.server_time - Utc::now()).num_milliseconds();
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.ready = true;
            state.heartbeat_interval = u64::from(ack.heartbeat_interval_seconds);
        }
        self.health.set_hello_ack(Utc::now(), offset);
        self.health.clear(codes::AUTH_REJECTED);
        self.health.set_auth_rejected(false);
        if offset.abs() > 5 * 60 * 1000 {
            self.health.raise(codes::CLOCK_SKEW, Severity::Warn, format!("clock offset {offset} ms"));
        } else {
            self.health.clear(codes::CLOCK_SKEW);
        }
        tracing::info!(offset_ms = offset, heartbeat = ack.heartbeat_interval_seconds, "session ready");

        // Vaciar el outbox y reenviar el último estado de cada tarea no *settled* (W6).
        agent.flush_outbox().await;
        agent.resend_unsettled().await;
    }

    async fn goodbye(&self, client: &AsyncClient, reason: &str) {
        let session_id = self.session_id();
        let goodbye = PresenceGoodbye { session_id, reason: reason.to_owned() };
        self.publish_presence(client, mt::PRESENCE_GOODBYE, &goodbye).await;
        let _ = client.disconnect().await;
        self.mark_disconnected();
    }

    /// Heartbeats periódicos (y uno inmediato cuando cambia `busy` o el estado de sesión).
    pub async fn heartbeats(self: Arc<Self>, agent: Arc<Agent>, cancel: CancellationToken) {
        let mut last_busy = false;
        let mut last_session = String::new();
        loop {
            let interval = self.heartbeat_interval();
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(interval.min(Duration::from_secs(5))) => {}
            }
            if !self.is_ready() {
                continue;
            }
            let busy = agent.busy();
            let session_state = self.session_state();
            let changed = busy != last_busy || session_state != last_session;
            let due = self.last_heartbeat_elapsed() >= interval;
            if !changed && !due {
                continue;
            }
            last_busy = busy;
            last_session.clone_from(&session_state);
            let client = self.state.lock().unwrap_or_else(PoisonError::into_inner).client.clone();
            if let Some(client) = client {
                let heartbeat = PresenceHeartbeat {
                    session_id: self.session_id(),
                    busy,
                    open_task_count: u32::try_from(agent.open_task_ids().len()).unwrap_or(0),
                    session_state: Some(session_state),
                };
                self.publish_presence(&client, mt::PRESENCE_HEARTBEAT, &heartbeat).await;
                self.mark_heartbeat();
            }
        }
    }

    fn last_heartbeat_elapsed(&self) -> Duration {
        let last = LAST_HEARTBEAT.lock().unwrap_or_else(PoisonError::into_inner);
        last.map_or(Duration::MAX, |instant| instant.elapsed())
    }

    fn mark_heartbeat(&self) {
        *LAST_HEARTBEAT.lock().unwrap_or_else(PoisonError::into_inner) = Some(std::time::Instant::now());
    }

    /// Apagado ordenado: `presence.goodbye` + DISCONNECT limpio (sin LWT).
    pub async fn shutdown(&self, reason: &str) {
        let client = self.state.lock().unwrap_or_else(PoisonError::into_inner).client.clone();
        if let Some(client) = client {
            self.goodbye(&client, reason).await;
        }
    }
}

static LAST_HEARTBEAT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

enum SessionEnd {
    AuthRejected,
    Cancelled,
    /// Alguien pidió reconectar ya (reanudación del equipo, `provision`).
    ReconnectRequested,
    Disconnected(String),
}
