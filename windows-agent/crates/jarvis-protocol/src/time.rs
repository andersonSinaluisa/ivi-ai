//! Marcas de tiempo del protocolo: ISO-8601 UTC con milisegundos (`2026-09-19T15:04:05.123Z`).

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serializer};

/// Formatea una marca de tiempo como la escribe el agente.
#[must_use]
pub fn format(ts: &DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Serde para `DateTime<Utc>`: escribe con milisegundos y acepta cualquier offset RFC 3339 al leer
/// (el Cloud .NET escribe `+00:00` y hasta 7 decimales).
pub mod millis {
    use super::{DateTime, Deserialize, Deserializer, Serializer, Utc};

    /// # Errors
    /// Propaga errores del serializer.
    pub fn serialize<S: Serializer>(ts: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::format(ts))
    }

    /// # Errors
    /// Falla si el texto no es RFC 3339.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&text).map(|dt| dt.with_timezone(&Utc)).map_err(serde::de::Error::custom)
    }
}

/// Igual que [`millis`] para campos opcionales.
pub mod millis_opt {
    use super::{DateTime, Deserialize, Deserializer, Serializer, Utc};

    /// # Errors
    /// Propaga errores del serializer.
    pub fn serialize<S: Serializer>(ts: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error> {
        match ts {
            Some(ts) => serializer.serialize_str(&super::format(ts)),
            None => serializer.serialize_none(),
        }
    }

    /// # Errors
    /// Falla si el texto no es RFC 3339.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        text.map(|t| {
            DateTime::parse_from_rfc3339(&t).map(|dt| dt.with_timezone(&Utc)).map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}
