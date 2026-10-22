//! Firmas Ed25519 de tareas y de decisiones de aprobación (HU-WIN-061, HU-WIN-062).
//!
//! Dos reglas sostienen esto:
//!
//! - lo que se firma es una **cadena canónica explícita**, no el JSON recibido: así dos agentes
//!   que reciben el mismo mensaje con distinto orden de claves o espacios calculan lo mismo;
//! - el payload entra por su **hash**, no por su texto, para que la cadena firmada tenga tamaño
//!   acotado y el payload no se pueda alterar sin invalidar la firma.
//!
//! La firma la produce el originador (el Cloud para tareas, el dispositivo del owner para
//! aprobaciones), nunca el agente: aquí solo se verifica.

use chrono::{DateTime, SecondsFormat, Utc};
use ed25519_dalek::{Signature as RawSignature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::messages::RequestedBy;

/// Prefijo de la cadena firmada de una tarea (versiona el formato).
pub const TASK_SCHEMA: &str = "jarvis.task-signature/1";
/// Prefijo de la cadena firmada de una decisión de aprobación.
pub const DECISION_SCHEMA: &str = "jarvis.approval-decision/1";

/// Firma tal y como viaja en el mensaje.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    /// Único algoritmo aceptado: `Ed25519`.
    pub alg: String,
    /// Qué clave pública hay que usar (debe estar registrada en `policy.toml`).
    pub key_id: String,
    /// Firma en base64 estándar (64 bytes).
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("unsupported signature algorithm")]
    UnsupportedAlgorithm,
    #[error("unknown key id")]
    UnknownKey,
    #[error("malformed signature")]
    Malformed,
    #[error("the signature does not match the message")]
    Invalid,
}

/// Clave pública registrada localmente.
#[derive(Debug, Clone)]
pub struct PublicKey {
    pub id: String,
    key: VerifyingKey,
}

impl PublicKey {
    /// Construye la clave desde 32 bytes en base64.
    ///
    /// # Errors
    /// Si no son 32 bytes válidos de una clave Ed25519.
    pub fn from_base64(id: impl Into<String>, encoded: &str) -> Result<Self, SignatureError> {
        let bytes = decode_base64(encoded).ok_or(SignatureError::Malformed)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| SignatureError::Malformed)?;
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| SignatureError::Malformed)?;
        Ok(Self { id: id.into(), key })
    }
}

/// Hash del payload en su forma canónica (claves ordenadas, sin espacios).
#[must_use]
pub fn payload_hash(payload: &Value) -> String {
    format!("{:x}", Sha256::digest(canonical_json(payload).as_bytes()))
}

/// JSON canónico: objetos con claves ordenadas y sin espacios.
///
/// Es la forma de [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) para los tipos que usa el
/// protocolo (objetos, listas, cadenas, booleanos, nulos y números enteros). Los números decimales
/// se serializan como los escribe `serde_json`, que no aplica la normalización de ECMAScript para
/// exponentes: los payloads del protocolo no los usan, y si algún día los usaran habría que fijar
/// esa normalización con el Cloud antes de firmarlos.
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            // Orden por unidades UTF-16, como pide RFC 8785.
            keys.sort_by_key(|key| key.encode_utf16().collect::<Vec<u16>>());
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*key).clone()).to_string());
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Cadena que firma el originador de una tarea.
#[must_use]
pub fn task_message(
    task_id: Uuid,
    task_type: &str,
    payload: &Value,
    requested_by: &RequestedBy,
    expires_at: DateTime<Utc>,
) -> String {
    format!(
        "{TASK_SCHEMA}\ntaskId={task_id}\ntype={task_type}\npayloadSha256={}\nrequestedBy={}:{}\nexpiresAt={}",
        payload_hash(payload),
        requested_by.kind,
        requested_by.id,
        timestamp(expires_at)
    )
}

/// Cadena que firma el dispositivo del owner al decidir una aprobación.
#[must_use]
pub fn decision_message(
    task_id: Uuid,
    decision: &str,
    payload_sha256: &str,
    nonce: &str,
    decided_at: DateTime<Utc>,
) -> String {
    format!(
        "{DECISION_SCHEMA}\ntaskId={task_id}\ndecision={decision}\npayloadSha256={payload_sha256}\nnonce={nonce}\ndecidedAt={}",
        timestamp(decided_at)
    )
}

/// Verifica una firma contra las claves registradas.
///
/// # Errors
/// [`SignatureError`] explicando por qué no vale; el llamante lo convierte en `NOT_PERMITTED`.
pub fn verify(message: &str, signature: &Signature, keys: &[PublicKey]) -> Result<String, SignatureError> {
    if !signature.alg.eq_ignore_ascii_case("ed25519") {
        return Err(SignatureError::UnsupportedAlgorithm);
    }
    let key = keys.iter().find(|key| key.id == signature.key_id).ok_or(SignatureError::UnknownKey)?;
    let bytes = decode_base64(&signature.value).ok_or(SignatureError::Malformed)?;
    let bytes: [u8; 64] = bytes.try_into().map_err(|_| SignatureError::Malformed)?;
    let raw = RawSignature::from_bytes(&bytes);
    key.key.verify(message.as_bytes(), &raw).map_err(|_| SignatureError::Invalid)?;
    Ok(key.id.clone())
}

/// Base64 estándar con relleno opcional (sin dependencias: el alfabeto es fijo y pequeño).
#[must_use]
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut accumulator: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = ALPHABET.iter().position(|candidate| *candidate == byte)? as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((accumulator >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

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

    fn keypair() -> (SigningKey, PublicKey) {
        // Clave fija: el test no necesita aleatoriedad y así es reproducible.
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public = PublicKey::from_base64("owner-phone-1", &encode_base64(signing.verifying_key().as_bytes()))
            .expect("public key");
        (signing, public)
    }

    fn sign(signing: &SigningKey, message: &str) -> Signature {
        Signature {
            alg: "Ed25519".to_owned(),
            key_id: "owner-phone-1".to_owned(),
            value: encode_base64(&signing.sign(message.as_bytes()).to_bytes()),
        }
    }

    #[test]
    fn canonical_json_orders_keys_and_drops_whitespace() {
        let a = serde_json::from_str::<Value>(r#"{ "b": 1, "a": { "y": true, "x": [1, 2] } }"#).expect("json");
        let b = serde_json::from_str::<Value>(r#"{"a":{"x":[1,2],"y":true},"b":1}"#).expect("json");
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(canonical_json(&a), r#"{"a":{"x":[1,2],"y":true},"b":1}"#);
        assert_eq!(payload_hash(&a), payload_hash(&b));
    }

    #[test]
    fn a_valid_signature_is_accepted_and_names_its_key() {
        let (signing, public) = keypair();
        let message = task_message(
            Uuid::nil(),
            "OPEN_APP",
            &json!({"appId": "chrome"}),
            &RequestedBy { kind: "User".into(), id: Uuid::nil() },
            Utc::now(),
        );
        let signature = sign(&signing, &message);
        assert_eq!(verify(&message, &signature, &[public]).expect("valid"), "owner-phone-1");
    }

    #[test]
    fn changing_any_signed_field_breaks_the_signature() {
        let (signing, public) = keypair();
        let expires = Utc::now();
        let requested_by = RequestedBy { kind: "User".into(), id: Uuid::nil() };
        let payload = json!({"appId": "chrome"});
        let original = task_message(Uuid::nil(), "OPEN_APP", &payload, &requested_by, expires);
        let signature = sign(&signing, &original);

        let other_id = Uuid::from_u128(1);
        let tampered = [
            task_message(other_id, "OPEN_APP", &payload, &requested_by, expires),
            task_message(Uuid::nil(), "CLOSE_APP", &payload, &requested_by, expires),
            task_message(Uuid::nil(), "OPEN_APP", &json!({"appId": "keepass"}), &requested_by, expires),
            task_message(
                Uuid::nil(),
                "OPEN_APP",
                &payload,
                &RequestedBy { kind: "Device".into(), id: Uuid::nil() },
                expires,
            ),
            task_message(Uuid::nil(), "OPEN_APP", &payload, &requested_by, expires + chrono::Duration::seconds(1)),
        ];
        for message in tampered {
            assert_eq!(
                verify(&message, &signature, std::slice::from_ref(&public)),
                Err(SignatureError::Invalid),
                "alterar un campo firmado debe invalidar: {message}"
            );
        }
    }

    #[test]
    fn unknown_keys_and_other_algorithms_are_refused() {
        let (signing, public) = keypair();
        let message = decision_message(Uuid::nil(), "GRANTED", "abc", "nonce-1", Utc::now());
        let mut signature = sign(&signing, &message);

        signature.key_id = "another-phone".to_owned();
        assert_eq!(verify(&message, &signature, std::slice::from_ref(&public)), Err(SignatureError::UnknownKey));

        signature.key_id = "owner-phone-1".to_owned();
        signature.alg = "RS256".to_owned();
        assert_eq!(
            verify(&message, &signature, std::slice::from_ref(&public)),
            Err(SignatureError::UnsupportedAlgorithm)
        );

        signature.alg = "Ed25519".to_owned();
        signature.value = "not base64!!".to_owned();
        assert_eq!(verify(&message, &signature, std::slice::from_ref(&public)), Err(SignatureError::Malformed));
    }

    #[test]
    fn the_wire_format_is_camel_case_like_the_rest_of_the_protocol() {
        // El Cloud publica `keyId`; con snake_case el agente rechazaba toda firma documentada.
        let raw = r#"{"alg":"Ed25519","keyId":"owner-phone-1","value":"AAA="}"#;
        let parsed: Signature = serde_json::from_str(raw).expect("la firma documentada debe parsear");
        assert_eq!(parsed.key_id, "owner-phone-1");
        assert_eq!(serde_json::to_string(&parsed).expect("serializa"), raw);
    }

    #[test]
    fn base64_round_trips() {
        for bytes in [vec![], vec![0u8], vec![1, 2], vec![1, 2, 3], (0u8..=255).collect::<Vec<u8>>()] {
            assert_eq!(decode_base64(&encode_base64(&bytes)), Some(bytes));
        }
    }
}
