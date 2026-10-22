//! Herramientas de memoria para el agente. Lo que guarda Qwen queda como "sin verificar" y con
//! la ejecución como fuente; el usuario puede revisarlo en la pantalla de Memoria.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::domain::{MemoryKind, NewMemory, NewRelation, RelationKind, RiskClass, ToolOutput, ToolSource, ToolSpec, Verification};
use crate::memory::memory_retriever::agent_source;
use crate::memory::{MemoryEngine, MemoryError, RetrievalOptions};
use crate::tools::{validate_object, Tool, ToolContext, ToolError};

const SAVE_SCHEMA: &str = include_str!("../../../../../../packages/tool-schemas/schemas/save_memory.json");
const SEARCH_SCHEMA: &str = include_str!("../../../../../../packages/tool-schemas/schemas/search_memory.json");

pub struct SaveMemory {
    spec: ToolSpec,
    engine: Arc<MemoryEngine>,
}

impl SaveMemory {
    #[must_use]
    pub fn new(engine: Arc<MemoryEngine>) -> Self {
        Self {
            spec: ToolSpec {
                id: "local.save_memory".into(),
                name: "save_memory".into(),
                description: "Guarda un recuerdo duradero del usuario (hecho, preferencia, procedimiento o entidad). Nunca contraseñas ni tokens.".into(),
                parameters: super::schema(SAVE_SCHEMA),
                risk: RiskClass::LowRisk,
                permissions: vec!["memory.write".into()],
                timeout_ms: 30_000,
                cancellable: false,
                source: ToolSource::Local,
                untrusted_output: false,
            },
            engine,
        }
    }
}

#[derive(Deserialize)]
struct SaveArgs {
    content: String,
    summary: Option<String>,
    kind: Option<MemoryKind>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    related_to: Vec<String>,
    relation: Option<RelationKind>,
}

#[async_trait]
impl Tool for SaveMemory {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate_object(&self.spec.parameters, arguments).map_err(ToolError::InvalidArguments)?;
        let args: SaveArgs = serde_json::from_value(arguments.clone()).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        if args.content.trim().is_empty() {
            return Err(ToolError::InvalidArguments("content está vacío".into()));
        }
        if matches!(args.kind, Some(MemoryKind::Episodic | MemoryKind::Experience)) {
            return Err(ToolError::InvalidArguments("kind no permitido".into()));
        }
        Ok(arguments.clone())
    }

    async fn execute(&self, arguments: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let args: SaveArgs = serde_json::from_value(arguments).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let kind = args.relation.unwrap_or(RelationKind::RelatedTo);
        let mut relations = Vec::new();
        let mut unknown = Vec::new();
        for short in &args.related_to {
            match self.engine.repository().resolve_prefix(short.trim_start_matches("m:").trim_matches(['[', ']'])) {
                Ok(Some(id)) => relations.push(NewRelation { target_id: id, kind }),
                _ => unknown.push(short.clone()),
            }
        }
        let new = NewMemory {
            kind: args.kind.unwrap_or(MemoryKind::Semantic),
            content: args.content,
            summary: args.summary,
            tags: args.tags,
            confidence: 0.7,
            verification: Verification::Unverified,
            retention: crate::domain::RetentionPolicy::Permanent,
            source: agent_source(&ctx.run_id),
            relations,
        };
        match self.engine.remember(new).await {
            Ok(out) => Ok(ToolOutput::success(json!({
                "saved": out.created,
                "duplicate": !out.created,
                "id": &out.memory.id[..8],
                "relations": out.relations.len(),
                "unknownRelatedIds": unknown,
                "note": "Guardado como no verificado: el usuario puede revisarlo en Memoria.",
            }))),
            Err(MemoryError::ContainsSecret) => Err(ToolError::NotPermitted("parece contener un secreto; no se guarda".into())),
            Err(e) => Err(ToolError::Failed(e.to_string())),
        }
    }
}

pub struct SearchMemory {
    spec: ToolSpec,
    engine: Arc<MemoryEngine>,
}

impl SearchMemory {
    #[must_use]
    pub fn new(engine: Arc<MemoryEngine>) -> Self {
        Self {
            spec: ToolSpec {
                id: "local.search_memory".into(),
                name: "search_memory".into(),
                description: "Busca en la memoria de Jarvis recuerdos relacionados con una consulta.".into(),
                parameters: super::schema(SEARCH_SCHEMA),
                risk: RiskClass::ReadOnly,
                permissions: vec!["memory.read".into()],
                timeout_ms: 30_000,
                cancellable: false,
                source: ToolSource::Local,
                // Los recuerdos importados podrían traer texto de terceros.
                untrusted_output: true,
            },
            engine,
        }
    }
}

#[async_trait]
impl Tool for SearchMemory {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate_object(&self.spec.parameters, arguments).map_err(ToolError::InvalidArguments)?;
        Ok(arguments.clone())
    }

    async fn execute(&self, arguments: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = arguments.get("query").and_then(Value::as_str).unwrap_or_default().to_owned();
        let limit = arguments.get("limit").and_then(Value::as_u64).map_or(5, |l| l.clamp(1, 10) as usize);
        let opts = RetrievalOptions { limit, run_id: Some(ctx.run_id.clone()), ..RetrievalOptions::default() };
        let r = self.engine.retrieve(&query, &opts).await.map_err(|e| ToolError::Failed(e.to_string()))?;
        let items: Vec<Value> = r
            .items
            .iter()
            .map(|i| {
                json!({
                    "id": &i.memory.id[..8],
                    "kind": i.memory.kind,
                    "content": crate::memory::memory_retriever::sanitize_for_prompt(&i.memory.content, 400),
                    "confidence": i.memory.confidence,
                    "verification": i.memory.verification,
                })
            })
            .collect();
        Ok(ToolOutput::success(json!({"results": items, "semantic": r.semantic})))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolContext {
        ToolContext { run_id: "run-1".into(), call_id: "c".into(), cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn the_agent_saves_unverified_memories_linked_by_short_ids() {
        let engine = Arc::new(MemoryEngine::for_tests().expect("e"));
        let loop_ = engine.remember(NewMemory::user(MemoryKind::Entity, "Proyecto LOOP")).await.expect("l").memory;
        let tool = SaveMemory::new(engine.clone());
        let args = json!({"content": "LOOP se despliega en un VPS", "related_to": [format!("m:{}", &loop_.id[..8])], "relation": "BELONGS_TO"});
        tool.validate(&args).expect("valid");
        let out = tool.execute(args, &ctx()).await.expect("ok");
        assert_eq!(out.data["saved"], true);
        let detail = engine.memory_detail(&engine.repository().resolve_prefix(out.data["id"].as_str().expect("id")).expect("r").expect("id")).expect("d");
        assert_eq!(detail.memory.verification, Verification::Unverified);
        assert_eq!(detail.source.expect("s").reference.as_deref(), Some("run-1"));
        assert!(detail.relations.iter().any(|r| r.relation.kind == RelationKind::BelongsTo && r.other.id == loop_.id));

        let secret = tool.execute(json!({"content": "su password: hunter222"}), &ctx()).await;
        assert!(matches!(secret, Err(ToolError::NotPermitted(_))));
        assert!(tool.validate(&json!({"content": "x", "kind": "episodic"})).is_err());
    }

    #[tokio::test]
    async fn search_returns_short_ids_and_logs_the_run() {
        let engine = Arc::new(MemoryEngine::for_tests().expect("e"));
        engine.remember(NewMemory::user(MemoryKind::Semantic, "Mi perro se llama Rocky")).await.expect("m");
        let out = SearchMemory::new(engine.clone()).execute(json!({"query": "¿cómo se llama mi perro?"}), &ctx()).await.expect("ok");
        assert_eq!(out.data["results"][0]["content"], "Mi perro se llama Rocky");
        assert_eq!(out.data["results"][0]["id"].as_str().expect("id").len(), 8);
        assert_eq!(engine.repository().events(None, Some("run-1"), 5).expect("e")[0].kind, "MEMORY_RETRIEVED");
    }
}
