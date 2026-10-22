//! Firmador del lado del cliente (ADR-012 del Cloud).
//!
//! Esta herramienta **no es parte del agente**: pertenece al owner. Existe porque la firma es
//! end-to-end y hoy no hay todavía una Owner Console que guarde la clave privada y firme desde ahí.
//! Mientras llegue, sirve para firmar tareas y decisiones de aprobación desde la línea de comandos
//! y para probar la cadena completa contra el Cloud real.
//!
//! La clave privada nunca sale de aquí: el Cloud recibe solo la firma, y quien la verifica es el
//! agente contra la clave pública que el owner instaló en su `policy.toml`.
//!
//! ```text
//! jarvis-sign keygen --key-id owner-laptop-1 --out owner.key
//! jarvis-sign task --key owner.key --target <uuid> --type OPEN_APP --payload '{"appId":"chrome"}' \
//!     --requested-by User:<uuid> --expires-in 300
//! jarvis-sign decision --key owner.key --task-id <uuid> --payload '{"appId":"chrome"}' --granted
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use jarvis_protocol::messages::RequestedBy;
use jarvis_protocol::signature;
use serde_json::{Value, json};
use uuid::Uuid;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Parser)]
#[command(name = "jarvis-sign", about = "Firma tareas y aprobaciones de Jarvis (cliente del owner)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Genera un par de claves nuevo e imprime el bloque para `policy.toml`.
    Keygen {
        #[arg(long)]
        key_id: String,
        /// Dónde guardar la clave privada (permisos del usuario; nunca se comparte).
        #[arg(long)]
        out: PathBuf,
    },
    /// Firma una tarea e imprime el cuerpo listo para `POST /api/v1/tasks`.
    Task {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        target: Uuid,
        #[arg(long = "type")]
        task_type: String,
        /// Payload JSON de la tarea.
        #[arg(long, default_value = "{}")]
        payload: String,
        /// Payload desde un archivo, para no pelearse con el escapado del shell.
        #[arg(long, conflicts_with = "payload")]
        payload_file: Option<PathBuf>,
        /// Quién pide la tarea, como `User:<uuid>` o `Device:<uuid>`.
        #[arg(long)]
        requested_by: String,
        /// Segundos hasta la expiración (se convierte a la marca absoluta que se firma).
        #[arg(long, default_value_t = 300)]
        expires_in: i64,
        /// Id de la tarea; por defecto uno nuevo. Va dentro de la firma.
        #[arg(long)]
        task_id: Option<Uuid>,
    },
    /// Firma una decisión de aprobación e imprime el cuerpo para `POST /tasks/{id}/approval`.
    Decision {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        task_id: Uuid,
        /// El payload **de esa tarea**: la firma queda atada a él.
        #[arg(long, default_value = "{}")]
        payload: String,
        /// Payload desde un archivo, para no pelearse con el escapado del shell.
        #[arg(long, conflicts_with = "payload")]
        payload_file: Option<PathBuf>,
        #[arg(long)]
        granted: bool,
        #[arg(long)]
        denied: bool,
        /// Nonce de un solo uso; por defecto uno aleatorio.
        #[arg(long)]
        nonce: Option<String>,
    },
}

fn encode_base64(bytes: &[u8]) -> String {
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

/// Lee `<keyId>:<semilla en base64>` del archivo de clave.
fn load_key(path: &PathBuf) -> Result<(String, SigningKey)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("no se puede leer {}", path.display()))?;
    let (key_id, seed) = text.trim().split_once(':').context("formato esperado: <keyId>:<semilla base64>")?;
    let bytes = signature::decode_base64(seed).context("la semilla no es base64")?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("la semilla no tiene 32 bytes"))?;
    Ok((key_id.to_owned(), SigningKey::from_bytes(&bytes)))
}

/// El payload llega inline o por archivo: en PowerShell las comillas de un JSON inline no
/// sobreviven al paso a un proceso nativo, y el archivo evita ese campo de minas.
fn read_payload(inline: &str, file: Option<&PathBuf>) -> Result<Value> {
    let text = match file {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("no se puede leer {}", path.display()))?,
        None => inline.to_owned(),
    };
    // El BOM que antepone PowerShell al escribir archivos no es JSON valido.
    serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).context("el payload no es JSON valido")
}

fn parse_requested_by(value: &str) -> Result<RequestedBy> {
    let (kind, id) = value.split_once(':').context("formato esperado: User:<uuid> o Device:<uuid>")?;
    if !matches!(kind, "User" | "Device") {
        bail!("el tipo debe ser User o Device");
    }
    Ok(RequestedBy { kind: kind.to_owned(), id: Uuid::parse_str(id).context("uuid invalido")? })
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn signature_json(key_id: &str, signing: &SigningKey, message: &str) -> Value {
    json!({
        "alg": "Ed25519",
        "keyId": key_id,
        "value": encode_base64(&signing.sign(message.as_bytes()).to_bytes()),
    })
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Keygen { key_id, out } => {
            use rand::RngCore;
            let mut seed = [0u8; 32];
            rand::rng().fill_bytes(&mut seed);
            let signing = SigningKey::from_bytes(&seed);
            std::fs::write(&out, format!("{key_id}:{}\n", encode_base64(&seed)))
                .with_context(|| format!("no se puede escribir {}", out.display()))?;
            // La publica aparte, en texto plano: es lo que se pega en policy.toml y lo que leen
            // los scripts. No es secreta.
            let public_path = out.with_extension("pub");
            let public = encode_base64(signing.verifying_key().as_bytes());
            std::fs::write(&public_path, format!("{public}\n"))
                .with_context(|| format!("no se puede escribir {}", public_path.display()))?;

            println!("Clave privada guardada en {} (no la compartas ni la subas al Cloud).\n", out.display());
            println!("Instala la publica en el policy.toml del agente:\n");
            println!("# tareas firmadas por este principal");
            println!("[[principals]]");
            println!("id = \"cloud:user:<tu userId>\"");
            println!(
                "public_keys = [{{ id = \"{key_id}\", key = \"{}\" }}]\n",
                encode_base64(signing.verifying_key().as_bytes())
            );
            println!("# y, si va a aprobar en remoto:");
            println!("[signatures]");
            println!("approver_keys = [{{ id = \"{key_id}\", key = \"{public}\" }}]");
        }
        Command::Task { key, target, task_type, payload, payload_file, requested_by, expires_in, task_id } => {
            let (key_id, signing) = load_key(&key)?;
            let payload = read_payload(&payload, payload_file.as_ref())?;
            let requested_by = parse_requested_by(&requested_by)?;
            let task_id = task_id.unwrap_or_else(Uuid::now_v7);
            let expires_at = Utc::now() + chrono::Duration::seconds(expires_in);

            let message = signature::task_message(task_id, &task_type, &payload, &requested_by, expires_at);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "taskId": task_id,
                    "targetDeviceId": target,
                    "type": task_type,
                    "payload": payload,
                    "expiresAt": timestamp(expires_at),
                    "signature": signature_json(&key_id, &signing, &message),
                }))?
            );
        }
        Command::Decision { key, task_id, payload, payload_file, granted, denied, nonce } => {
            if granted == denied {
                bail!("indica exactamente una de --granted o --denied");
            }
            let (key_id, signing) = load_key(&key)?;
            let payload = read_payload(&payload, payload_file.as_ref())?;
            let decision = if granted { "GRANTED" } else { "DENIED" };
            let nonce = nonce.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
            let decided_at = Utc::now();

            let message =
                signature::decision_message(task_id, decision, &signature::payload_hash(&payload), &nonce, decided_at);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "decision": decision,
                    "nonce": nonce,
                    "decidedAt": timestamp(decided_at),
                    "signature": signature_json(&key_id, &signing, &message),
                }))?
            );
        }
    }
    Ok(())
}
