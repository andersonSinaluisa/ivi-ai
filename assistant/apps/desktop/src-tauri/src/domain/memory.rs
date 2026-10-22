//! Tipos de la memoria de Jarvis: recuerdos, fuentes, relaciones y habilidades.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $s),+ }
            }
            #[must_use]
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$variant),)+ _ => None }
            }
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// Lo que pasó.
    Episodic,
    /// Hechos y preferencias.
    Semantic,
    /// Cómo se hace algo (habilidades).
    Procedural,
    /// Resultado de una tarea registrada.
    Experience,
    /// Cosa con nombre (proyecto, persona, herramienta) que agrupa recuerdos.
    Entity,
}
str_enum!(MemoryKind { Episodic => "episodic", Semantic => "semantic", Procedural => "procedural", Experience => "experience", Entity => "entity" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Verification {
    #[default]
    Unverified,
    Verified,
    /// El usuario lo marcó como dudoso o incorrecto: no se usa en la recuperación.
    Disputed,
}
str_enum!(Verification { Unverified => "unverified", Verified => "verified", Disputed => "disputed" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RetentionPolicy {
    Permanent,
    Days { days: u32 },
    /// Se borra en la siguiente consolidación.
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    User,
    Agent,
    Conversation,
    Tool,
    Import,
    Experience,
    /// Sugerido por la consolidación del Cloud Core.
    Consolidation,
    /// Recibido de otro device sin más detalle.
    Device,
}
str_enum!(SourceKind {
    User => "user",
    Agent => "agent",
    Conversation => "conversation",
    Tool => "tool",
    Import => "import",
    Experience => "experience",
    Consolidation => "consolidation",
    Device => "device",
});

/// Procedencia de un recuerdo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySource {
    pub id: String,
    pub kind: SourceKind,
    /// Referencia concreta: id de conversación, de ejecución, de instalación…
    pub reference: Option<String>,
    pub label: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    /// UUID estable (se conserva al exportar e importar).
    pub id: String,
    pub kind: MemoryKind,
    pub content: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Etiqueta legible de la fuente.
    pub source: String,
    pub source_id: Option<String>,
    /// 0..1, cuánto suele importar.
    pub relevance: f32,
    /// 0..1, cuán seguro es que sea cierto.
    pub confidence: f32,
    pub verification: Verification,
    pub tags: Vec<String>,
    /// Referencias heredadas del esquema v1 (las nuevas van en `memory_relations`).
    pub references: Vec<String>,
    pub retention: RetentionPolicy,
    #[serde(default)]
    pub content_hash: String,
}

impl MemoryRecord {
    /// ¿Ya venció según su política de retención?
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match self.retention {
            RetentionPolicy::Permanent => false,
            RetentionPolicy::Session => true,
            RetentionPolicy::Days { days } => now - self.created_at > chrono::Duration::days(i64::from(days)),
        }
    }

    /// Texto corto para listas y nodos del grafo.
    #[must_use]
    pub fn label(&self) -> String {
        let base = if self.summary.trim().is_empty() { &self.content } else { &self.summary };
        let one_line = base.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.chars().count() > 80 { one_line.chars().take(79).collect::<String>() + "…" } else { one_line }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationKind {
    BelongsTo,
    DependsOn,
    CausedBy,
    SolvedBy,
    RelatedTo,
    LearnedFrom,
    UsedIn,
    /// Mismo contenido que otro recuerdo (sugerido por la consolidación; nunca verificado solo).
    DuplicateOf,
}
str_enum!(RelationKind {
    BelongsTo => "BELONGS_TO",
    DependsOn => "DEPENDS_ON",
    CausedBy => "CAUSED_BY",
    SolvedBy => "SOLVED_BY",
    RelatedTo => "RELATED_TO",
    LearnedFrom => "LEARNED_FROM",
    UsedIn => "USED_IN",
    DuplicateOf => "DUPLICATE_OF",
});

/// Quién afirmó la relación. Una relación `inferred` nunca se considera verificada por sí sola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    User,
    Agent,
    Inferred,
    Import,
    /// Creada por la consolidación del Cloud Core: sugerencia, nunca verificada sola.
    Consolidation,
}
str_enum!(Provenance { User => "user", Agent => "agent", Inferred => "inferred", Import => "import", Consolidation => "consolidation" });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub kind: RelationKind,
    pub confidence: f32,
    pub provenance: Provenance,
    pub verified: bool,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Datos para crear un recuerdo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    pub kind: MemoryKind,
    pub content: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub verification: Verification,
    #[serde(default = "default_retention")]
    pub retention: RetentionPolicy,
    pub source: NewSource,
    /// Relaciones explícitas con recuerdos existentes.
    #[serde(default)]
    pub relations: Vec<NewRelation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSource {
    pub kind: SourceKind,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRelation {
    pub target_id: String,
    pub kind: RelationKind,
}

fn default_confidence() -> f32 {
    0.8
}

fn default_retention() -> RetentionPolicy {
    RetentionPolicy::Permanent
}

impl NewMemory {
    #[must_use]
    pub fn user(kind: MemoryKind, content: impl Into<String>) -> Self {
        Self {
            kind,
            content: content.into(),
            summary: None,
            tags: Vec::new(),
            confidence: 1.0,
            verification: Verification::Verified,
            retention: RetentionPolicy::Permanent,
            source: NewSource { kind: SourceKind::User, reference: None, label: Some("usuario".into()) },
            relations: Vec::new(),
        }
    }
}

/// Cambios permitidos desde la UI.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPatch {
    pub content: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<Vec<String>>,
    pub confidence: Option<f32>,
    pub verification: Option<Verification>,
    pub retention: Option<RetentionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub procedure: String,
    /// Una habilidad importada empieza desactivada y pendiente de revisión.
    pub enabled: bool,
    pub requires_review: bool,
    pub source: String,
    pub experience_id: Option<String>,
    pub current_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillVersion {
    pub skill_id: String,
    pub version: i64,
    pub procedure: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}
