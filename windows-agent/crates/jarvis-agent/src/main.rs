//! `jarvis-agent`: proceso residente y CLI local (docs/01-architecture/component-model.md#cli-local).

use std::path::PathBuf;

use jarvis_agent::{
    cloud, config, diagnostics, health, install, ipc, logging, paths, provision, runtime, tools, update,
};

use clap::{Parser, Subcommand};
use jarvis_policy::{Policy, PolicyState, Principal};
use serde_json::Value;
use uuid::Uuid;

use crate::cloud::AGENT_VERSION;
use crate::config::Config;
use crate::ipc::{Command as IpcCommand, Response};
use crate::paths::Paths;

#[derive(Debug, Parser)]
#[command(name = "jarvis-agent", version, about = "Jarvis Windows Agent", disable_version_flag = true)]
struct Cli {
    /// Raíz de datos alternativa (por defecto `%LOCALAPPDATA%\Jarvis\Agent`).
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Ejecuta el agente (lo usa la Scheduled Task al iniciar sesión).
    Run {
        /// Logs legibles por consola en lugar de JSON en archivo.
        #[arg(long)]
        console: bool,
        /// Permite arrancar con token elevado (solo para pruebas).
        #[arg(long)]
        allow_elevated: bool,
    },
    /// Guarda la credencial del device (secreto por consola o stdin) y prueba la conexión.
    Provision {
        #[arg(long)]
        device_id: Uuid,
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = 8883)]
        port: u16,
        /// MQTT sin TLS: solo permitido para hosts loopback (entorno de desarrollo).
        #[arg(long)]
        plaintext: bool,
        /// Lee el secreto de stdin (automatización). Nunca se acepta como argumento.
        #[arg(long)]
        secret_stdin: bool,
        /// CA propia (PEM) para brokers de desarrollo.
        #[arg(long)]
        ca_file: Option<PathBuf>,
        /// Crea `policy.toml` concediendo al owner las capabilities de procesos.
        #[arg(long)]
        owner_user_id: Option<Uuid>,
        /// No escribir `policy.toml`.
        #[arg(long)]
        no_policy: bool,
    },
    /// Muestra el health snapshot del agente en ejecución.
    Status {
        #[arg(long)]
        json: bool,
        /// Levanta el modo seguro tras un crash loop (HU-WIN-018).
        #[arg(long)]
        clear_safe_mode: bool,
        /// Da por visto el aviso de que la base local se recreó (HU-WIN-043).
        #[arg(long)]
        ack_reset: bool,
    },
    /// Apaga el agente de forma ordenada.
    Stop,
    /// Fuerza una reconexión al Cloud (tras reprovisionar o si la red cambió).
    Reconnect,
    /// Tools registradas y su estado.
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Valida la política local y explica decisiones concretas.
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    /// Aprobaciones locales pendientes.
    Approvals {
        #[command(subcommand)]
        command: ApprovalsCommand,
    },
    /// Ejecuta una tool localmente por el mismo pipeline (sin Cloud); para desarrollo y pruebas.
    Invoke {
        #[arg(long)]
        tool: String,
        /// Input JSON de la tool.
        #[arg(long, default_value = "{}")]
        input: String,
    },
    /// Instala el agente para este usuario y registra la Scheduled Task de inicio de sesión.
    Install {
        /// Arranca la tarea al terminar.
        #[arg(long)]
        start: bool,
        /// Permite instalar desde un proceso elevado (solo pruebas).
        #[arg(long)]
        allow_elevated: bool,
    },
    /// Quita la tarea, la credencial y los binarios.
    Uninstall {
        /// Borra también configuración, política, base de datos y logs.
        #[arg(long)]
        purge: bool,
        /// No preguntar.
        #[arg(long)]
        yes: bool,
    },
    /// Genera un paquete de soporte redactado.
    Diagnostics {
        /// Días de logs a incluir.
        #[arg(long, default_value_t = 3)]
        days: i64,
        /// Excluye los logs.
        #[arg(long)]
        no_logs: bool,
        /// Muestra el contenido por consola en lugar de escribirlo.
        #[arg(long)]
        print: bool,
    },
    /// Busca e instala una versión nueva firmada (HU-WIN-063).
    Update {
        /// Solo comprobar si hay versión nueva, sin descargarla.
        #[arg(long)]
        check: bool,
        /// Manifiesto local en lugar de `update.manifest_url` (equipos sin salida a internet).
        #[arg(long)]
        manifest_file: Option<std::path::PathBuf>,
        /// Cambia la tarea a la versión nueva y reinicia el agente al terminar.
        #[arg(long)]
        apply: bool,
    },
    /// Versión y protocolo.
    Version,
}

#[derive(Debug, Subcommand)]
enum ToolsCommand {
    List,
}

#[derive(Debug, Subcommand)]
enum PolicyCommand {
    /// Recarga `policy.toml` en el agente en ejecución (sin reiniciar).
    Reload,
    /// Valida `policy.toml`; con argumentos, muestra la decisión para una invocación.
    Check {
        #[arg(long)]
        principal: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long, default_value = "{}")]
        input: String,
    },
}

#[derive(Debug, Subcommand)]
enum ApprovalsCommand {
    List,
    Approve { task_id: Uuid },
    Deny { task_id: Uuid },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve(cli.root.clone())?;
    paths.ensure()?;

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(async move { dispatch(cli, paths).await })
}

async fn dispatch(cli: Cli, paths: Paths) -> anyhow::Result<()> {
    match cli.command {
        Commands::Run { console, allow_elevated } => run(paths, console, allow_elevated).await,
        Commands::Provision { device_id, host, port, plaintext, secret_stdin, ca_file, owner_user_id, no_policy } => {
            provision::run(
                &paths,
                provision::Options {
                    device_id,
                    host,
                    port,
                    plaintext,
                    secret_stdin,
                    ca_file,
                    owner_user_id,
                    no_policy,
                },
            )
            .await
        }
        Commands::Status { json, clear_safe_mode, ack_reset } => status(&paths, json, clear_safe_mode, ack_reset).await,
        Commands::Stop => {
            let response = send(&paths, &IpcCommand::Stop).await?;
            print_response(&response);
            Ok(())
        }
        Commands::Reconnect => {
            let response = send(&paths, &IpcCommand::Reconnect).await?;
            print_response(&response);
            Ok(())
        }
        Commands::Tools { command: ToolsCommand::List } => tools_list(&paths),
        Commands::Policy { command: PolicyCommand::Check { principal, tool, input } } => {
            policy_check(&paths, principal, tool, &input)
        }
        Commands::Policy { command: PolicyCommand::Reload } => {
            print_response(&send(&paths, &IpcCommand::PolicyReload).await?);
            Ok(())
        }
        Commands::Approvals { command } => approvals(&paths, command).await,
        Commands::Invoke { tool, input } => invoke(paths, &tool, &input).await,
        Commands::Install { start, allow_elevated } => install::install(
            &paths,
            &install::InstallOptions { root_override: cli.root.clone(), start, allow_elevated },
        ),
        Commands::Uninstall { purge, yes } => {
            install::uninstall(&paths, install::UninstallOptions { purge, yes }).await
        }
        Commands::Diagnostics { days, no_logs, print } => {
            diagnostics::assert_config_is_shareable(&paths)?;
            let options = diagnostics::Options { days, include_logs: !no_logs, print };
            if let Some(path) = diagnostics::collect(&paths, options)? {
                println!("Diagnostics written to {}", path.display());
                println!("Review the files before sharing them: they include your config, policy and logs.");
            }
            Ok(())
        }
        Commands::Update { check, manifest_file, apply } => {
            let config = Config::load(&paths.config())?;
            let Some(staged) = update::run(&paths, &config.update, manifest_file.as_deref(), check)? else {
                return Ok(());
            };
            if !apply {
                println!("Run `jarvis-agent update --apply` to switch to it, or keep using the current version.");
                return Ok(());
            }
            // El cambio de versión lo decide el owner, y solo con el agente parado.
            let store = jarvis_state::Store::open(&paths.database())?;
            let version = update::Version::parse(
                staged
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .as_str(),
            )?;
            update::activate(&store, &staged, &version, cli.root.as_deref())?;
            println!("The scheduled task now points to {version}. Restart the agent to use it:");
            println!("  jarvis-agent stop && schtasks /run /tn \"{}\"", jarvis_windows::task_scheduler::TASK_NAME);
            println!("If it does not start and connect, the agent rolls back to the previous version by itself.");
            Ok(())
        }
        Commands::Version => {
            println!("{AGENT_VERSION} (protocol v1, presence {})", cloud::PROTOCOL_VERSION);
            Ok(())
        }
    }
}

async fn run(paths: Paths, console: bool, allow_elevated: bool) -> anyhow::Result<()> {
    let config = Config::load(&paths.config())?;
    let _guard = logging::init(&paths.logs(), &config.logging.level, console)?;
    logging::prune(&paths.logs(), config.logging.retention_days);

    // Instancia única por usuario: dos agentes competirían por la misma base y credencial.
    #[cfg(windows)]
    let _instance = {
        let sid = jarvis_windows::user_sid_string().map_err(|e| anyhow::anyhow!("cannot read the user SID: {e}"))?;
        jarvis_windows::SingleInstance::acquire(&sid).map_err(|e| anyhow::anyhow!("{e}"))?
    };

    let runtime = runtime::build(&paths, config, allow_elevated)?;
    runtime.run().await
}

async fn send(paths: &Paths, command: &IpcCommand) -> anyhow::Result<Response> {
    let _ = paths;
    #[cfg(windows)]
    {
        let sid = jarvis_windows::user_sid_string().map_err(|e| anyhow::anyhow!("cannot read the user SID: {e}"))?;
        ipc::send(&paths::pipe_name(&sid), command).await
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        anyhow::bail!("the local channel is only available on Windows")
    }
}

fn print_response(response: &Response) {
    match response {
        Response::Ok { message } => println!("{message}"),
        Response::Error { message } => println!("error: {message}"),
        other => println!("{}", serde_json::to_string_pretty(other).unwrap_or_default()),
    }
}

async fn status(paths: &Paths, json: bool, clear_safe_mode: bool, ack_reset: bool) -> anyhow::Result<()> {
    if clear_safe_mode {
        print_response(&send(paths, &IpcCommand::ClearSafeMode).await?);
    }
    if ack_reset {
        print_response(&send(paths, &IpcCommand::AckStateReset).await?);
    }
    let response = match send(paths, &IpcCommand::Status).await {
        Ok(response) => response,
        Err(error) => {
            println!("agent not running ({error})");
            std::process::exit(2);
        }
    };
    let Response::Status(snapshot) = response else {
        print_response(&response);
        std::process::exit(2);
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        let uptime = snapshot.agent.uptime_seconds;
        println!(
            "Jarvis Windows Agent {}   {:?} (uptime {}h {}m)",
            snapshot.agent.version,
            snapshot.agent.state,
            uptime / 3600,
            (uptime % 3600) / 60
        );
        println!("Device     {}", snapshot.agent.device_id.clone().unwrap_or_else(|| "not provisioned".into()));
        println!(
            "Cloud      {} {}   hello.ack {}   clock {} ms",
            if snapshot.mqtt.connected { "connected" } else { "disconnected" },
            snapshot.mqtt.broker.clone().unwrap_or_default(),
            snapshot.cloud.last_hello_ack.map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_else(|| "-".into()),
            snapshot.cloud.clock_offset_ms
        );
        println!("Session    {}", snapshot.session.state);
        println!(
            "Tasks      running {} · awaiting approval {} · outbox {} · unsettled {}",
            snapshot.tools.running, snapshot.tools.awaiting_approval, snapshot.db.outbox_pending, snapshot.db.unsettled
        );
        println!("Policy     {}", if snapshot.policy.valid { "valid" } else { "INVALID (fail-closed)" });
        if snapshot.conditions.is_empty() {
            println!("Conditions none");
        } else {
            for condition in &snapshot.conditions {
                println!("Condition  [{:?}] {} — {}", condition.severity, condition.code, condition.detail);
            }
        }
    }
    let worst = snapshot.conditions.iter().map(|c| c.severity).max();
    match worst {
        Some(health::Severity::Error) => std::process::exit(2),
        Some(health::Severity::Warn) => std::process::exit(1),
        None => Ok(()),
    }
}

fn load_policy(paths: &Paths) -> (Policy, bool, Option<String>) {
    match Policy::load(&paths.policy()) {
        Ok(policy) => (policy, true, None),
        Err(error) => (Policy::default(), false, Some(error.to_string())),
    }
}

fn tools_list(paths: &Paths) -> anyhow::Result<()> {
    let config = Config::load(&paths.config())?;
    let (policy, _, _) = load_policy(paths);
    let api = platform_api(&config)?;
    let ctx = tools::ToolContext::new(api, std::sync::Arc::new(policy));
    let registry = tools::build_registry(&ctx, &config.tools.enabled).map_err(|e| anyhow::anyhow!(e))?;
    println!("TOOL                 TASK TYPE          RISK  ENABLED   PERMISSIONS");
    for (descriptor, enabled) in registry.descriptors() {
        println!(
            "{:<20} {:<18} {:<5} {:<9} {}",
            descriptor.name,
            descriptor.name.to_uppercase(),
            descriptor.risk_level.as_str(),
            if enabled { "yes" } else { "no" },
            descriptor.required_permissions.join(", ")
        );
    }
    println!("\nsupportedTaskTypes announced to the Cloud: {}", registry.supported_task_types().join(", "));
    Ok(())
}

fn policy_check(paths: &Paths, principal: Option<String>, tool: Option<String>, input: &str) -> anyhow::Result<()> {
    let (policy, valid, error) = load_policy(paths);
    if let Some(error) = error {
        println!("policy.toml is INVALID: {error}");
        println!("The agent would run fail-closed (only R0 tools).");
    } else {
        println!("policy.toml is valid ({} principals, {} deny rules)", policy.principals.len(), policy.deny.len());
    }

    let (Some(principal), Some(tool_name)) = (principal, tool) else { return Ok(()) };
    let principal = Principal::parse(&principal).ok_or_else(|| {
        anyhow::anyhow!("unknown principal (use cloud:user:<uuid>, cloud:device:<uuid>, local:owner)")
    })?;
    let input: Value = serde_json::from_str(input)?;

    let config = Config::load(&paths.config())?;
    let api = platform_api(&config)?;
    let policy = std::sync::Arc::new(policy);
    let ctx = tools::ToolContext::new(api, policy.clone());
    let registry = tools::build_registry(&ctx, &config.tools.enabled).map_err(|e| anyhow::anyhow!(e))?;
    let resolved = registry
        .resolve_tool(&tool_name, input)
        .ok_or_else(|| anyhow::anyhow!("unknown or disabled tool '{tool_name}'"))?;
    let validated = resolved.tool.validate(&resolved.input).map_err(|e| anyhow::anyhow!("invalid input: {e}"))?;
    let risk = resolved.tool.risk(&validated);
    let permissions = resolved.tool.required_permissions(&validated);
    let state = PolicyState { policy: (*policy).clone(), valid };
    let decision = {
        // Se simula el caso real de hoy: el Cloud no firma todavía, así que una identidad remota es
        // solo afirmada. Si la política exige firma, aquí se ve el `SIGNATURE_REQUIRED`.
        let authenticity = if principal.is_remote() {
            jarvis_policy::Authenticity::Claimed
        } else {
            jarvis_policy::Authenticity::Local
        };
        jarvis_policy::evaluate(
            &state,
            &principal,
            resolved.tool.descriptor().name,
            &validated,
            risk,
            &permissions,
            &authenticity,
        )
    };

    println!("\nprincipal   {principal}");
    println!("tool        {} (risk {risk})", resolved.tool.descriptor().name);
    println!("permissions {}", permissions.join(", "));
    println!("decision    {}", decision.kind.as_str());
    println!("reason      {}", decision.reason_code);
    if principal.is_remote() {
        println!("assuming    an unsigned request (the Cloud does not sign yet)");
    }
    if !decision.message.is_empty() {
        println!("message     {}", decision.message);
    }
    println!("rule        {}", decision.rule.unwrap_or_else(|| "-".into()));
    Ok(())
}

async fn approvals(paths: &Paths, command: ApprovalsCommand) -> anyhow::Result<()> {
    let command = match command {
        ApprovalsCommand::List => IpcCommand::ApprovalsList,
        ApprovalsCommand::Approve { task_id } => IpcCommand::ApprovalsDecide { task_id, granted: true },
        ApprovalsCommand::Deny { task_id } => IpcCommand::ApprovalsDecide { task_id, granted: false },
    };
    let response = send(paths, &command).await?;
    match response {
        Response::Approvals(items) if items.is_empty() => println!("no pending approvals"),
        Response::Approvals(items) => {
            println!("TASK                                   TOOL         RISK  REQUESTED BY                 INPUT");
            for item in items {
                println!(
                    "{:<38} {:<12} {:<5} {:<28} {}",
                    item.task_id, item.tool, item.risk_level, item.principal, item.input_summary
                );
            }
            println!("\nApprove with: jarvis-agent approvals approve <TASK>");
        }
        other => print_response(&other),
    }
    Ok(())
}

async fn invoke(paths: Paths, tool: &str, input: &str) -> anyhow::Result<()> {
    let config = Config::load(&paths.config())?;
    let _guard = logging::init(&paths.logs(), &config.logging.level, true)?;
    let input: Value = serde_json::from_str(input)?;
    let runtime = runtime::build(&paths, config, false)?;
    // Misma política, mismos locks, misma verificación: una invocación local no salta controles.
    let result = runtime.agent.invoke_local(tool, input, &Principal::LocalOwner).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    if result.outcome == jarvis_tooling::Outcome::Succeeded { Ok(()) } else { std::process::exit(1) }
}

fn platform_api(config: &Config) -> anyhow::Result<std::sync::Arc<dyn jarvis_tooling::platform::WindowsApi>> {
    #[cfg(windows)]
    {
        Ok(std::sync::Arc::new(jarvis_windows::Win32Api::new(config.idle_threshold())?))
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        anyhow::bail!("the agent only runs on Windows")
    }
}
