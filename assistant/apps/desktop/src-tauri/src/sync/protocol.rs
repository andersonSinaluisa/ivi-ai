//! Contratos de `jarvis.sync/1` (api/docs/03-protocols/sync-protocol.md). Los ejemplos normativos están en
//! `packages/agent-protocol/schemas/sync-vectors.json`, copia exacta del archivo del Cloud Core.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL: &str = "jarvis.sync/1";
pub const MAX_CHANGES_PER_PUSH: usize = 200;
/// Margen bajo el límite de 1 MiB del Cloud.
pub const MAX_PUSH_BYTES: usize = 900 * 1024;

pub mod entity {
    pub const MEMORY: &str = "memory";
    pub const RELATION: &str = "relation";
    pub const EMBEDDING: &str = "embedding";
    pub const EXPERIENCE: &str = "experience";
    pub const SKILL: &str = "skill";
    pub const ALL: [&str; 5] = [MEMORY, RELATION, EMBEDDING, EXPERIENCE, SKILL];
}

pub mod status {
    pub const APPLIED: &str = "applied";
    pub const MERGED: &str = "merged";
    pub const CONFLICT: &str = "conflict";
    pub const REJECTED: &str = "rejected";
}

pub const ALL_SCOPES: [&str; 6] =
    ["memories:read", "memories:write", "experiences:read", "experiences:write", "skills:read", "skills:write"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionData {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceData {
    pub kind: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryData {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "half")]
    pub relevance: f32,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default = "unverified")]
    pub verification: String,
    pub retention: RetentionData,
    pub source: SourceData,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationData {
    pub source_id: String,
    pub target_id: String,
    pub kind: String,
    #[serde(default = "half")]
    pub confidence: f32,
    #[serde(default = "user")]
    pub provenance: String,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingData {
    pub model: String,
    pub dims: usize,
    pub content_hash: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceStepData {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub verification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceData {
    pub goal: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub steps: Vec<ExperienceStepData>,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub evidence: Vec<Value>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub procedure: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub completed: bool,
    #[serde(default = "one")]
    pub occurrences: u32,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillCompatibility {
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub required_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillPermissions {
    #[serde(default)]
    pub max_risk: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillData {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub procedure: String,
    #[serde(default = "draft")]
    pub status: String,
    #[serde(default)]
    pub compatibility: Option<SkillCompatibility>,
    #[serde(default)]
    pub permissions: Option<SkillPermissions>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub experience_id: Option<String>,
    #[serde(default)]
    pub changelog: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub change_id: String,
    pub entity: String,
    pub entity_id: String,
    pub op: String,
    pub base_version: Option<i64>,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub protocol: String,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub change_id: String,
    pub status: String,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub version: Option<i64>,
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub conflict_id: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub conflict_kind: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResponse {
    pub results: Vec<PushResult>,
    pub cursor: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub protocol: String,
    pub since: i64,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    #[serde(default)]
    pub embedding_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PulledChange {
    pub seq: i64,
    pub entity: String,
    pub entity_id: String,
    pub op: String,
    pub version: i64,
    #[serde(default)]
    pub origin_device_id: Option<String>,
    #[serde(default)]
    pub change_id: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResponse {
    pub changes: Vec<PulledChange>,
    pub next_since: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDto {
    pub id: String,
    pub entity: String,
    pub entity_id: String,
    pub conflict_kind: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub base_version: Option<i64>,
    pub server_version: i64,
    #[serde(default)]
    pub server_data: Option<Value>,
    #[serde(default)]
    pub device_data: Option<Value>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    pub resolution: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryEmbedding {
    pub model: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<QueryEmbedding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand_graph: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    pub version: i64,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub confidence: f32,
    pub verification: String,
    pub score: f64,
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationDto {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub kind: String,
    pub confidence: f32,
    pub provenance: String,
    pub verified: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub method: String,
    pub memories: Vec<SearchHit>,
    pub relations: Vec<RelationDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthFeatures {
    pub sync: String,
    pub vector_search: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub api_version: String,
    pub server_time: DateTime<Utc>,
    pub features: HealthFeatures,
}

// ---- Auth y devices (endpoints existentes del Cloud Core) -----------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessToken {
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredDevice {
    pub device: DeviceRef,
    pub credential: DeviceCredential,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredential {
    pub device_id: String,
    pub secret: String,
}

fn half() -> f32 {
    0.5
}
fn default_confidence() -> f32 {
    0.8
}
fn unverified() -> String {
    "unverified".into()
}
fn user() -> String {
    "user".into()
}
fn draft() -> String {
    "draft".into()
}
fn one() -> u32 {
    1
}

/// SHA-256 en hex del contenido exacto (UTF-8, sin normalizar): valida que un vector sea de esta versión.
#[must_use]
pub fn content_sha256(content: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vectors_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/agent-protocol/schemas/sync-vectors.json")
    }

    fn vectors() -> Value {
        serde_json::from_str(&std::fs::read_to_string(vectors_path()).expect("sync-vectors.json")).expect("json")
    }

    /// Igualdad semántica: nulos = ausentes, números redondeados, instantes normalizados.
    fn normalize(v: &Value) -> Value {
        match v {
            Value::Object(map) => Value::Object(
                map.iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k.clone(), normalize(x))).collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
            Value::Number(n) => n.as_f64().map_or(v.clone(), |f| serde_json::json!((f * 100_000.0).round() / 100_000.0)),
            Value::String(s) => DateTime::parse_from_rfc3339(s)
                .map_or(v.clone(), |d| Value::String(d.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())),
            _ => v.clone(),
        }
    }

    fn round_trip<T: Serialize + for<'de> Deserialize<'de>>(name: &str) {
        let example = vectors()[name].clone();
        let parsed: T = serde_json::from_value(example.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(normalize(&serde_json::to_value(parsed).expect("ser")), normalize(&example), "{name}");
    }

    #[test]
    fn examples_round_trip_with_the_cloud_contract() {
        assert_eq!(vectors()["protocol"], PROTOCOL);
        round_trip::<PushRequest>("pushRequest");
        round_trip::<PushResponse>("pushResponse");
        round_trip::<PullRequest>("pullRequest");
        round_trip::<PullResponse>("pullResponse");
        round_trip::<ConflictDto>("conflict");
        round_trip::<ResolveRequest>("resolveRequest");
        round_trip::<SearchRequest>("searchRequest");
        round_trip::<SearchResponse>("searchResponse");
        round_trip::<HealthResponse>("healthResponse");
        let entities = &vectors()["entities"];
        for (name, check) in [
            ("memory", serde_json::from_value::<MemoryData>(entities["memory"].clone()).map(|_| ())),
            ("memoryWithDaysRetention", serde_json::from_value::<MemoryData>(entities["memoryWithDaysRetention"].clone()).map(|_| ())),
            ("relation", serde_json::from_value::<RelationData>(entities["relation"].clone()).map(|_| ())),
            ("embedding", serde_json::from_value::<EmbeddingData>(entities["embedding"].clone()).map(|_| ())),
            ("experience", serde_json::from_value::<ExperienceData>(entities["experience"].clone()).map(|_| ())),
            ("skill", serde_json::from_value::<SkillData>(entities["skill"].clone()).map(|_| ())),
        ] {
            check.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn content_hash_matches_the_cloud() {
        for case in vectors()["contentHash"].as_array().expect("array") {
            assert_eq!(content_sha256(case["content"].as_str().expect("s")), case["sha256"].as_str().expect("s"));
        }
    }

    /// La copia del escritorio debe ser idéntica a la del Cloud Core cuando ambos están en `ivi/`.
    #[test]
    fn vectors_are_the_same_file_as_in_the_cloud_core() {
        let cloud = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../api/docs/03-protocols/sync-vectors.json");
        if let Ok(original) = std::fs::read(&cloud) {
            assert_eq!(std::fs::read(vectors_path()).expect("copia"), original, "copia desactualizada de sync-vectors.json");
        }
    }
}
