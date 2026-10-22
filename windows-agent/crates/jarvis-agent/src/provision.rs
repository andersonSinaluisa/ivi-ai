//! `jarvis-agent provision`: guarda la credencial del device y prepara la configuración local.
//!
//! El secreto se pide por consola (sin eco) o por stdin: **nunca** como argumento de la línea de
//! comandos, que es visible para otros procesos y queda en historiales (ADR-009).

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use jarvis_policy::Policy;
use uuid::Uuid;

use crate::config::{BrokerUrl, Config};
use crate::paths::Paths;

pub struct Options {
    pub device_id: Uuid,
    pub host: String,
    pub port: u16,
    pub plaintext: bool,
    pub secret_stdin: bool,
    pub ca_file: Option<PathBuf>,
    pub owner_user_id: Option<Uuid>,
    pub no_policy: bool,
}

/// Provisiona el agente y comprueba la conexión antes de guardar nada.
///
/// # Errors
/// Credencial inválida, broker inalcanzable o error al escribir la configuración.
pub async fn run(paths: &Paths, options: Options) -> anyhow::Result<()> {
    let scheme = if options.plaintext { "mqtt" } else { "mqtts" };
    let url = format!("{scheme}://{}:{}", options.host, options.port);
    let broker = BrokerUrl::parse(&url).map_err(|e| anyhow::anyhow!(e))?;

    let secret = read_secret(options.secret_stdin)?;
    if secret.trim().is_empty() {
        anyhow::bail!("the device secret is empty");
    }

    println!("Testing the connection to {url} …");
    verify_connection(options.device_id, &broker, options.ca_file.as_deref(), &secret).await?;
    println!("Connection OK.");

    store_secret(options.device_id, &secret)?;
    println!("Secret stored in Windows Credential Manager (Jarvis/WindowsAgent/{}).", options.device_id);

    let mut config = Config::load(&paths.config())?;
    config.device.device_id = Some(options.device_id);
    config.cloud.broker_url = Some(url);
    config.cloud.ca_file = options.ca_file.clone();
    std::fs::write(paths.config(), config.to_toml()?)?;
    println!("Configuration written to {}", paths.config().display());

    if !options.no_policy && !paths.policy().exists() {
        std::fs::write(paths.policy(), Policy::owner_template(options.owner_user_id))?;
        println!("Initial policy written to {}", paths.policy().display());
        if options.owner_user_id.is_none() {
            println!(
                "No remote principal was granted: the agent will reject Cloud tasks until you add one \
                 (`[[principals]] id = \"cloud:user:<id>\"`)."
            );
        }
    }

    // Si el agente está corriendo, que reconecte con la credencial nueva.
    #[cfg(windows)]
    if let Ok(sid) = jarvis_windows::user_sid_string()
        && crate::ipc::send(&crate::paths::pipe_name(&sid), &crate::ipc::Command::Reconnect).await.is_ok()
    {
        println!("The running agent was asked to reconnect.");
    }
    Ok(())
}

fn read_secret(from_stdin: bool) -> anyhow::Result<String> {
    if from_stdin {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        // PowerShell antepone un BOM a stdin: sin quitarlo el secreto seria otro y el broker
        // rechazaria la conexion sin explicar por que (visto al automatizar el provisionado).
        return Ok(buffer.trim_start_matches('\u{feff}').trim().to_owned());
    }
    let secret = rpassword::prompt_password("Device secret: ")?;
    Ok(secret.trim().to_owned())
}

#[cfg(windows)]
fn store_secret(device_id: Uuid, secret: &str) -> anyhow::Result<()> {
    use jarvis_windows::{CredentialStore, Secret, WindowsCredentialStore};
    WindowsCredentialStore
        .put(&device_id.hyphenated().to_string(), &Secret::new(secret))
        .map_err(|e| anyhow::anyhow!("cannot store the credential: {e}"))
}

#[cfg(not(windows))]
fn store_secret(_device_id: Uuid, _secret: &str) -> anyhow::Result<()> {
    anyhow::bail!("provisioning is only available on Windows")
}

/// Conecta al broker con esas credenciales: si la autenticación falla, no se guarda nada.
async fn verify_connection(
    device_id: Uuid,
    broker: &BrokerUrl,
    ca_file: Option<&std::path::Path>,
    secret: &str,
) -> anyhow::Result<()> {
    use rumqttc::{AsyncClient, ConnectReturnCode, Event, MqttOptions, Packet};

    let id = device_id.hyphenated().to_string();
    let mut options = MqttOptions::new(id.clone(), &broker.host, broker.port);
    options.set_credentials(id, secret.to_owned());
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    if broker.tls {
        let config = crate::cloud::tls_client_config(ca_file)?;
        options.set_transport(rumqttc::Transport::Tls(rumqttc::TlsConfiguration::Rustls(std::sync::Arc::new(config))));
    }

    let (client, mut eventloop) = AsyncClient::new(options, 8);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let event = tokio::time::timeout_at(deadline, eventloop.poll())
            .await
            .map_err(|_| anyhow::anyhow!("the broker did not answer within 15s"))?;
        match event {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                let _ = client.disconnect().await;
                return match ack.code {
                    ConnectReturnCode::Success => Ok(()),
                    ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized => {
                        Err(anyhow::anyhow!("the broker rejected the credential; nothing was stored"))
                    }
                    other => Err(anyhow::anyhow!("CONNACK {other:?}")),
                };
            }
            Ok(_) => {}
            Err(error) => return Err(anyhow::anyhow!("cannot connect: {error}")),
        }
    }
}
