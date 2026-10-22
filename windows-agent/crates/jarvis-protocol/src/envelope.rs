//! Envelope común (mqtt-protocol.md §3).

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tamaño máximo de un mensaje (límite del broker y del Cloud).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const ENVELOPE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub v: u32,
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(with = "crate::time::millis")]
    pub ts: DateTime<Utc>,
    pub device_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<Uuid>,
    pub data: serde_json::Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("message too large ({0} bytes)")]
    TooLarge(usize),
    #[error("invalid envelope: {0}")]
    Malformed(String),
    #[error("unsupported envelope version {0}")]
    UnsupportedVersion(u32),
    #[error("data must be a JSON object")]
    DataNotObject,
    #[error("invalid data for {kind}: {reason}")]
    InvalidData { kind: String, reason: String },
}

impl Envelope {
    /// Crea un envelope saliente con `id` UUIDv7 y `ts` = ahora.
    ///
    /// # Errors
    /// Falla solo si `data` no se puede serializar (no ocurre con los tipos del crate).
    pub fn create<T: Serialize>(
        kind: &str,
        device_id: Uuid,
        data: &T,
        correlation_id: Option<Uuid>,
        causation_id: Option<Uuid>,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            v: ENVELOPE_VERSION,
            id: Uuid::now_v7(),
            kind: kind.to_owned(),
            ts: Utc::now(),
            device_id,
            correlation_id,
            causation_id,
            data: serde_json::to_value(data)?,
        })
    }

    /// Parsea y valida un mensaje entrante: tamaño antes de parsear, campos requeridos, `v == 1`
    /// y `data` objeto. Los campos desconocidos se ignoran (compatibilidad hacia adelante).
    ///
    /// # Errors
    /// Devuelve [`EnvelopeError`] si el mensaje no cumple el contrato.
    pub fn parse(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(EnvelopeError::TooLarge(bytes.len()));
        }
        let envelope: Self = serde_json::from_slice(bytes).map_err(|e| EnvelopeError::Malformed(e.to_string()))?;
        if envelope.v != ENVELOPE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(envelope.v));
        }
        if !envelope.data.is_object() {
            return Err(EnvelopeError::DataNotObject);
        }
        Ok(envelope)
    }

    /// Deserializa `data` al tipo del mensaje.
    ///
    /// # Errors
    /// [`EnvelopeError::InvalidData`] si `data` no tiene la forma esperada.
    pub fn data_as<T: DeserializeOwned>(&self) -> Result<T, EnvelopeError> {
        serde_json::from_value(self.data.clone())
            .map_err(|e| EnvelopeError::InvalidData { kind: self.kind.clone(), reason: e.to_string() })
    }

    /// Lee `data.taskId` aunque el resto de `data` sea inválido (para poder responder `task.rejected`).
    #[must_use]
    pub fn task_id_hint(&self) -> Option<Uuid> {
        self.data.get("taskId")?.as_str().and_then(|s| Uuid::try_parse(s).ok())
    }

    /// Serializa a bytes JSON UTF-8.
    ///
    /// # Errors
    /// Solo si la serialización falla (no ocurre con valores JSON válidos).
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_before_parsing() {
        let big = vec![b' '; MAX_MESSAGE_BYTES + 1];
        assert_eq!(Envelope::parse(&big), Err(EnvelopeError::TooLarge(MAX_MESSAGE_BYTES + 1)));
    }

    #[test]
    fn rejects_other_versions_and_non_object_data() {
        let base = r#"{"id":"0192a6f4-9100-7c3a-9d2e-000000000020","type":"task.dispatch","ts":"2026-09-19T15:04:05.400Z","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002""#;
        let v2 = format!(r#"{base},"v":2,"data":{{}}}}"#);
        assert_eq!(Envelope::parse(v2.as_bytes()), Err(EnvelopeError::UnsupportedVersion(2)));
        let arr = format!(r#"{base},"v":1,"data":[]}}"#);
        assert_eq!(Envelope::parse(arr.as_bytes()), Err(EnvelopeError::DataNotObject));
        assert!(matches!(Envelope::parse(b"not json"), Err(EnvelopeError::Malformed(_))));
        let missing_id = r#"{"v":1,"type":"x","ts":"2026-09-19T15:04:05Z","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002","data":{}}"#;
        assert!(matches!(Envelope::parse(missing_id.as_bytes()), Err(EnvelopeError::Malformed(_))));
    }

    #[test]
    fn ignores_unknown_fields_and_accepts_dotnet_timestamps() {
        let json = r#"{"v":1,"id":"0192a6f4-9100-7c3a-9d2e-000000000020","type":"task.update.ack","ts":"2026-09-19T15:04:05.1234567+00:00","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002","future":true,"data":{"taskId":"0192a6f4-9000-7c3a-9d2e-00000000a001","status":"Completed"}}"#;
        let envelope = Envelope::parse(json.as_bytes()).expect("valid envelope");
        assert_eq!(envelope.kind, "task.update.ack");
        assert!(envelope.task_id_hint().is_some());
    }

    #[test]
    fn writes_millisecond_utc_timestamps_and_omits_nulls() {
        let envelope =
            Envelope::create("task.ack", Uuid::nil(), &serde_json::json!({}), None, None).expect("serializable");
        let text = String::from_utf8(envelope.to_bytes().expect("serializable")).expect("utf8");
        assert!(!text.contains("correlationId"));
        let ts = envelope.ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert!(text.contains(&ts), "{text}");
        assert!(ts.ends_with('Z'));
    }
}
