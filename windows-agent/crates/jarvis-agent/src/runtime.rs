//! Composición y ciclo de vida del proceso residente (docs/05-windows/startup-model.md).

use std::sync::Arc;
use std::time::Duration;

use jarvis_policy::{Policy, PolicyState};
use jarvis_state::Store;
use jarvis_tooling::{Executor, ExecutorConfig, ToolRegistry};
use tokio_util::sync::CancellationToken;

use crate::cloud::{AGENT_VERSION, CloudLink, SecretSource};
use crate::config::{BrokerUrl, Config};
use crate::health::{AgentState, Health, Severity, codes};
use crate::paths::Paths;
use crate::pipeline::{Agent, Publisher};
use crate::tools::{ToolContext, build_registry};

/// Avisa de aprobaciones pendientes con un globo del área de notificación.
#[cfg(windows)]
struct TrayNotifier {
    icon: jarvis_windows::notify::TrayIcon,
}

#[cfg(windows)]
impl crate::pipeline::ApprovalNotifier for TrayNotifier {
    fn approval_required(&self, tool: &str, risk: &str, task_id: uuid::Uuid) {
        let body = format!(
            "{tool} ({risk}) espera tu aprobación.
Ejecuta: jarvis-agent approvals approve {task_id}"
        );
        if let Err(error) = self.icon.balloon("Jarvis: aprobación pendiente", &body) {
            tracing::debug!(%error, "cannot show the approval balloon");
        }
    }
}

/// Publicador inerte: sin device provisionado no hay a dónde publicar (todo espera en el outbox).
struct NullPublisher;

#[async_trait::async_trait]
impl Publisher for NullPublisher {
    async fn publish(&self, _channel: jarvis_protocol::Channel, _payload: Vec<u8>) -> bool {
        false
    }
}

/// Credencial desde Windows Credential Manager (nunca desde archivos ni variables de entorno).
#[cfg(windows)]
struct CredentialManagerSecret {
    device_id: String,
}

#[cfg(windows)]
impl SecretSource for CredentialManagerSecret {
    fn secret(&self) -> Option<String> {
        use jarvis_windows::CredentialStore;
        jarvis_windows::WindowsCredentialStore
            .get(&self.device_id)
            .ok()
            .flatten()
            .map(|secret| secret.expose().to_owned())
    }
}

pub struct Runtime {
    pub agent: Arc<Agent>,
    pub tools: Arc<ToolContext>,
    pub health: Arc<Health>,
    pub link: Option<Arc<CloudLink>>,
    pub shutdown: CancellationToken,
    pub reconnect: Arc<tokio::sync::Notify>,
    pub config: Config,
    pub paths: Paths,
}

/// Construye todo el agente a partir de la configuración y la política locales.
///
/// # Errors
/// Configuración, política, base de datos o registro de tools inválidos (fail fast).
pub fn build(paths: &Paths, config: Config, allow_elevated: bool) -> anyhow::Result<Runtime> {
    let health = Arc::new(Health::new());

    // Endurecimiento del proceso antes de cargar nada opcional (WT-08, WT-12).
    #[cfg(windows)]
    {
        let report = jarvis_windows::hardening::apply();
        tracing::debug!(?report, "process hardening applied");
        if !report.dll_search_restricted {
            tracing::warn!("could not restrict the DLL search path");
        }
    }

    #[cfg(windows)]
    let elevated = jarvis_windows::is_elevated();
    #[cfg(not(windows))]
    let elevated = false;
    if elevated {
        health.raise(codes::RUNNING_ELEVATED, Severity::Warn, "the agent is running with an elevated token");
        if !allow_elevated {
            anyhow::bail!(
                "the agent must run as a standard user (ADR-011). Start it without elevation, \
                 or use --allow-elevated for testing."
            );
        }
    }

    // Estado local: si la base está corrupta se aparta y se crea una nueva (guardia STATE_RESET).
    let (store, was_reset) = Store::open_or_reset(&paths.database())?;
    if was_reset {
        health.raise(codes::STATE_RESET, Severity::Warn, "the local database was recreated");
    }
    let store = Arc::new(store);

    // Política local: inválida → fail-closed (solo R0) conservando el resto del agente operativo.
    let (policy, valid) = match Policy::load(&paths.policy()) {
        Ok(policy) => (policy, true),
        Err(error) => {
            tracing::error!(%error, "invalid policy.toml: only R0 tools will be allowed");
            health.raise(codes::POLICY_INVALID, Severity::Error, error.to_string());
            (Policy::default(), false)
        }
    };
    health.set_policy(valid);
    let policy = Arc::new(policy);

    #[cfg(windows)]
    let api: Arc<dyn jarvis_tooling::platform::WindowsApi> =
        Arc::new(jarvis_windows::Win32Api::new(config.idle_threshold())?);
    #[cfg(not(windows))]
    let api: Arc<dyn jarvis_tooling::platform::WindowsApi> = anyhow::bail!("the agent only runs on Windows");

    let ctx = ToolContext::new(api, policy.clone());
    let registry: ToolRegistry = build_registry(&ctx, &config.tools.enabled).map_err(|e| anyhow::anyhow!(e))?;
    let registry = Arc::new(registry);
    let executor = Arc::new(Executor::new(ExecutorConfig { max_concurrent: config.executor.max_concurrent }));
    let policy_state = Arc::new(PolicyState { policy: (*policy).clone(), valid });

    let broker =
        config.cloud.broker_url.as_deref().map(BrokerUrl::parse).transpose().map_err(|e| anyhow::anyhow!(e))?;
    let link = match (config.device.device_id, broker.clone()) {
        (Some(device_id), Some(broker)) => Some(CloudLink::new(
            device_id,
            broker,
            config.cloud.ca_file.clone(),
            config.mqtt.keepalive_seconds,
            config.mqtt.max_inflight,
            health.clone(),
        )),
        _ => {
            health.raise(codes::NOT_PROVISIONED, Severity::Error, "run `jarvis-agent provision`");
            None
        }
    };

    let publisher: Arc<dyn Publisher> =
        link.clone().map_or_else(|| Arc::new(NullPublisher) as Arc<dyn Publisher>, |link| link as Arc<dyn Publisher>);
    let device_id = config.device.device_id.unwrap_or_default();
    health.set_identity(
        config.device.device_id.map(|id| id.hyphenated().to_string()),
        elevated,
        config.cloud.broker_url.clone(),
    );

    let agent = Agent::new(device_id, store, registry, executor, policy_state, health.clone(), publisher);
    Ok(Runtime {
        agent,
        tools: ctx,
        health,
        link,
        shutdown: CancellationToken::new(),
        reconnect: Arc::new(tokio::sync::Notify::new()),
        config,
        paths: paths.clone(),
    })
}

impl Runtime {
    /// Arranca todas las tareas del agente y espera al apagado ordenado.
    ///
    /// # Errors
    /// Si el canal local no se puede crear.
    pub async fn run(self) -> anyhow::Result<()> {
        let Self { agent, tools, health, link, shutdown, reconnect, config, paths } = self;
        let policy_reloader =
            crate::policy_watch::PolicyReloader::new(paths.policy(), agent.clone(), tools, health.clone());
        health.set_agent_state(AgentState::Starting);

        // Una tool desbocada no debe ahogar la PC del usuario.
        #[cfg(windows)]
        let _limits = match jarvis_windows::hardening::limit_memory(512) {
            Ok(limits) => Some(limits),
            Err(error) => {
                tracing::warn!(%error, "cannot apply the memory limit to this process");
                None
            }
        };

        // Crash loop: si el proceso se ha caído varias veces seguidas, no se aceptan tareas nuevas.
        match crate::safe_mode::on_start(&agent.store, chrono::Utc::now()) {
            Ok(startup) if startup.safe_mode => {
                let reason = startup.reason.unwrap_or_else(|| "repeated crashes".to_owned());
                tracing::error!(%reason, "starting in SAFE MODE: new tasks will be rejected");
                agent.set_safe_mode(true);
                health.set_agent_state(AgentState::SafeMode);
                health.raise(codes::SAFE_MODE, Severity::Error, reason);
            }
            Ok(startup) if startup.unclean_starts > 0 => {
                tracing::warn!(unclean_starts = startup.unclean_starts, "the previous run did not shut down cleanly");
            }
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "cannot evaluate the crash-loop guard"),
        }

        // Cambio de versión en curso: contar el intento y, si esta versión no termina de arrancar
        // bien, volver sola a la anterior (HU-WIN-063).
        #[cfg(windows)]
        let update_watch = update_guard(&agent.store, &health, paths.root.clone())?;

        // Recuperación de arranque antes de conectar (W9).
        agent.recover().await;

        #[cfg(windows)]
        let user_sid =
            jarvis_windows::user_sid_string().map_err(|e| anyhow::anyhow!("cannot read the user SID: {e}"))?;
        #[cfg(not(windows))]
        let user_sid = "local".to_owned();

        let mut tasks = Vec::new();

        if let Some(link) = link.clone() {
            let device_id = config.device.device_id.unwrap_or_default();
            #[cfg(windows)]
            let secrets: Arc<dyn SecretSource> =
                Arc::new(CredentialManagerSecret { device_id: device_id.hyphenated().to_string() });
            #[cfg(not(windows))]
            let secrets: Arc<dyn SecretSource> = unreachable!();

            let link_task = {
                let (link, agent, cancel) = (link.clone(), agent.clone(), shutdown.clone());
                let reconnect = reconnect.clone();
                tokio::spawn(async move { link.run(agent, secrets, reconnect, cancel).await })
            };
            let heartbeat_task = {
                let (link, agent, cancel) = (link.clone(), agent.clone(), shutdown.clone());
                tokio::spawn(async move { link.heartbeats(agent, cancel).await })
            };
            tasks.push(link_task);
            tasks.push(heartbeat_task);
        }

        // Canal local de la CLI.
        #[cfg(windows)]
        {
            let handler = Arc::new(crate::ipc::IpcHandler {
                agent: agent.clone(),
                policy: policy_reloader.clone(),
                shutdown: shutdown.clone(),
                reconnect: reconnect.clone(),
                version: AGENT_VERSION,
            });
            let pipe = crate::paths::pipe_name(&user_sid);
            let cancel = shutdown.clone();
            let sid = user_sid.clone();
            tasks.push(tokio::spawn(async move { crate::ipc::serve(handler, pipe, sid, cancel).await }));
        }

        // Local Tool API para el Agent Core (EPIC-16). Apagada salvo que el owner la encienda.
        #[cfg(windows)]
        if config.tool_api.enabled {
            let api = crate::tool_api::ToolApi::new(agent.clone(), config.tool_api.max_calls_per_minute);
            let pipe = crate::tool_api::pipe_name(&user_sid);
            let (sid, cancel) = (user_sid.clone(), shutdown.clone());
            tracing::info!(pipe = %pipe, "local tool API enabled for local:agent-core");
            tasks.push(tokio::spawn(async move { crate::tool_api::serve(api, pipe, sid, cancel).await }));
        }

        // Avisos de Windows: apagado de sesión, bloqueo/desbloqueo y suspensión/reanudación.
        #[cfg(windows)]
        let _window = {
            let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
            let window = jarvis_windows::events::MessageWindow::spawn(events_tx)
                .map_err(|e| anyhow::anyhow!("cannot create the message window: {e}"))?;
            let (health_events, cancel, reconnect_events) = (health.clone(), shutdown.clone(), reconnect.clone());
            tasks.push(tokio::spawn(async move {
                loop {
                    let event = tokio::select! {
                        () = cancel.cancelled() => return,
                        event = events_rx.recv() => event,
                    };
                    let Some(event) = event else { return };
                    use jarvis_windows::events::SystemEvent;
                    match event {
                        SystemEvent::EndSessionRequested => {
                            tracing::info!("Windows is closing the session: shutting down");
                            cancel.cancel();
                            return;
                        }
                        SystemEvent::SessionLocked => health_events.set_session_state("Locked"),
                        SystemEvent::SessionUnlocked | SystemEvent::SessionConnected => {
                            health_events.set_session_state("Active");
                        }
                        SystemEvent::SessionDisconnected => health_events.set_session_state("Disconnected"),
                        SystemEvent::Suspending => tracing::info!("the computer is suspending"),
                        SystemEvent::Resumed => {
                            tracing::info!("resumed from suspend: reconnecting now");
                            // Con permiso guardado: si el enlace esta ocupado, la peticion espera
                            // a la siguiente vuelta en vez de perderse.
                            reconnect_events.notify_one();
                        }
                    }
                }
            }));
            // Aviso al usuario, sin botones: la decisión se toma con la CLI (WT-15).
            match jarvis_windows::notify::TrayIcon::new(window.handle(), "Jarvis Windows Agent") {
                Ok(icon) => agent.set_notifier(Arc::new(TrayNotifier { icon })),
                Err(error) => tracing::warn!(%error, "no notification area available; approvals only via CLI"),
            }
            window
        };

        // Mantenimiento: estado de sesión, aprobaciones vencidas, reenvíos y limpieza.
        tasks.push({
            let (agent, health, cancel) = (agent.clone(), health.clone(), shutdown.clone());
            let policy = policy_reloader.clone();
            let retention_days = 8;
            tokio::spawn(async move { maintenance(agent, health, policy, retention_days, cancel).await })
        });

        if !agent.safe_mode() {
            health.set_agent_state(AgentState::Running);
        }
        tracing::info!(version = AGENT_VERSION, root = %paths.root.display(), "agent started");

        // Ventana de confirmación de la versión nueva: o conecta a tiempo, o se vuelve atrás.
        #[cfg(windows)]
        if let Some(pending) = update_watch {
            let (store, health, cancel) = (agent.store.clone(), health.clone(), shutdown.clone());
            let root = paths.root.clone();
            tasks.push(tokio::spawn(async move {
                confirm_or_roll_back(store, health, pending, root, cancel).await;
            }));
        }

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::warn!(%error, "cannot listen for Ctrl+C");
                }
            }
            () = shutdown.cancelled() => {}
        }

        tracing::info!("shutting down");
        health.set_agent_state(AgentState::Stopping);
        if let Err(error) = crate::safe_mode::on_clean_shutdown(&agent.store) {
            tracing::warn!(%error, "cannot record the clean shutdown");
        }
        if let Some(link) = link {
            link.shutdown("shutdown").await;
        }
        shutdown.cancel();
        for task in tasks {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
        Ok(())
    }
}

/// Al arrancar con una actualización pendiente: suma el intento y decide si seguir probando esta
/// versión o volver a la anterior. Devuelve el registro pendiente si esta ejecución debe confirmarse.
#[cfg(windows)]
fn update_guard(
    store: &Arc<jarvis_state::Store>,
    health: &Arc<Health>,
    root: std::path::PathBuf,
) -> anyhow::Result<Option<crate::update::Pending>> {
    let Some(pending) = crate::update::pending(store)? else { return Ok(None) };

    if pending.version != AGENT_VERSION {
        // Se está ejecutando otra versión de la que se activó: o ya se volvió atrás, o alguien
        // cambió la tarea a mano. El registro deja de tener sentido.
        tracing::warn!(expected = pending.version, running = AGENT_VERSION, "stale update record: discarded");
        crate::update::confirm(store)?;
        return Ok(None);
    }

    let pending = crate::update::record_attempt(store, pending)?;
    if crate::update::should_roll_back(&pending) {
        tracing::error!(
            version = pending.version,
            attempts = pending.attempts,
            previous = pending.previous_version,
            "the new version did not start correctly: rolling back"
        );
        match crate::update::roll_back(store, &pending, Some(root.as_path())) {
            Ok(()) => health.raise(
                codes::UPDATE_ROLLED_BACK,
                Severity::Error,
                format!("rolled back to {} after {} failed starts", pending.previous_version, pending.attempts),
            ),
            Err(error) => {
                tracing::error!(%error, "the rollback could not be completed");
                health.raise(codes::UPDATE_ROLLED_BACK, Severity::Error, error.to_string());
            }
        }
        // El proceso sigue vivo para que el usuario vea el estado; la próxima sesión arrancará la
        // versión anterior, que es a la que ya apunta la tarea.
        return Ok(None);
    }

    tracing::info!(version = pending.version, attempt = pending.attempts, "running a version pending confirmation");
    Ok(Some(pending))
}

/// Confirma la versión nueva si llega a conectar dentro de la ventana; si no, vuelve a la anterior.
#[cfg(windows)]
async fn confirm_or_roll_back(
    store: Arc<jarvis_state::Store>,
    health: Arc<Health>,
    pending: crate::update::Pending,
    root: std::path::PathBuf,
    cancel: CancellationToken,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(crate::update::HEALTH_WINDOW_SECONDS);
    loop {
        if health.is_connected() {
            tracing::info!(version = pending.version, "the new version connected: update confirmed");
            if let Err(error) = crate::update::confirm(&store) {
                tracing::warn!(%error, "cannot record the update confirmation");
            }
            return;
        }
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::error!(
                version = pending.version,
                seconds = crate::update::HEALTH_WINDOW_SECONDS,
                "the new version did not connect within its window: rolling back"
            );
            match crate::update::roll_back(&store, &pending, Some(root.as_path())) {
                Ok(()) => health.raise(
                    codes::UPDATE_ROLLED_BACK,
                    Severity::Error,
                    format!("rolled back to {}: the new version did not connect", pending.previous_version),
                ),
                Err(error) => health.raise(codes::UPDATE_ROLLED_BACK, Severity::Error, error.to_string()),
            }
            return;
        }
    }
}

async fn maintenance(
    agent: Arc<Agent>,
    health: Arc<Health>,
    policy: Arc<crate::policy_watch::PolicyReloader>,
    retention_days: i64,
    cancel: CancellationToken,
) {
    let mut ticks: u64 = 0;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
        ticks += 1;

        #[cfg(windows)]
        {
            let state = jarvis_windows::session::state(Duration::from_secs(600));
            health.set_session_state(state.as_str());
        }

        agent.expire_approvals().await;
        // La política es la frontera de seguridad: se recarga sola cuando el owner la edita.
        policy.reload_if_changed();

        // Reenvío periódico de lo no confirmado (W6) y limpieza por retención.
        if ticks % 12 == 0 {
            agent.flush_outbox().await;
            agent.resend_unsettled().await;
            if let Ok(stats) = agent.store.stats() {
                if stats.oldest_unsettled_age_seconds.is_some_and(|age| age > 7 * 24 * 3600) {
                    health.raise(codes::OUTBOX_STUCK, Severity::Warn, "there are results older than 7 days");
                } else {
                    health.clear(codes::OUTBOX_STUCK);
                }
            }
        }
        if ticks % 720 == 0 {
            match agent.store.cleanup(retention_days, chrono::Utc::now()) {
                Ok(removed) if removed > 0 => tracing::info!(removed, "retention cleanup"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "cleanup failed"),
            }
        }
        if agent.executor.stuck() > 0 {
            health.raise(codes::TOOL_STUCK, Severity::Error, "a tool did not stop after its timeout");
        }
    }
}
