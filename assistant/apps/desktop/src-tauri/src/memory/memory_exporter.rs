//! Exportación de la memoria a un paquete cifrado `.jarvismem`.
//!
//! ```text
//! jarvis-memory/
//! ├── manifest.json     versiones, fecha, instalación, modelo de embeddings, habilidades, checksums
//! ├── memories.sqlite   solo tablas de memoria (sin conversaciones, auditoría ni ajustes)
//! ├── skills/<id>.json  cada habilidad con sus versiones (legible)
//! ├── attachments/      reservado para adjuntos
//! └── checksums.json    SHA-256 de todos los archivos anteriores
//! ```
//! No se exportan contraseñas, tokens ni credenciales: la memoria ya los rechaza al guardar, y la
//! evidencia de las experiencias se vuelve a limpiar antes de empaquetar.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::memory_engine::MemoryEngine;
use super::memory_package::{check_extension, seal, tar, KdfParams};
use super::memory_repository::{hex, storage};
use super::secrets::redact_secrets;
use super::{MemoryError, MemoryResult};
use crate::domain::{MemoryKind, Skill, SkillVersion};
use crate::infrastructure::persistence::{Database, SCHEMA_VERSION};

pub const FORMAT: &str = "jarvis-memory";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    /// Tipos a exportar (todos si es None).
    pub kinds: Option<Vec<MemoryKind>>,
    pub include_experiences: bool,
    pub include_skills: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { kinds: None, include_experiences: true, include_skills: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub memories: u64,
    pub relations: u64,
    pub sources: u64,
    pub embeddings: u64,
    pub experiences: u64,
    pub skills: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRef {
    pub id: String,
    pub name: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub schema_version: i64,
    pub app_version: String,
    pub exported_at: String,
    pub installation_id: String,
    pub embedding_model: String,
    pub counts: Counts,
    pub skills: Vec<SkillRef>,
    /// SHA-256 de cada archivo de datos (no incluye manifest.json ni checksums.json).
    pub checksums: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub bytes: u64,
    pub manifest: Manifest,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillFile {
    pub skill: Skill,
    pub versions: Vec<SkillVersion>,
}

pub(crate) fn sha256(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub(crate) fn count(conn: &Connection, table: &str) -> MemoryResult<u64> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).map_err(storage)?;
    Ok(u64::try_from(n).unwrap_or(0))
}

impl MemoryEngine {
    /// Ruta por defecto: `Documentos\Jarvis\jarvis-memory-AAAAMMDD-HHMMSS.jarvismem`.
    #[must_use]
    pub fn default_export_path(&self) -> PathBuf {
        self.config.exports_dir.join(format!("jarvis-memory-{}.jarvismem", Utc::now().format("%Y%m%d-%H%M%S")))
    }

    /// # Errors
    /// Ruta o contraseña inválidas, errores de E/S o de SQLite.
    pub async fn export_memory(&self, path: &Path, passphrase: &str, options: &ExportOptions, kdf: KdfParams) -> MemoryResult<ExportReport> {
        check_extension(path)?;
        let _guard = self.job_lock.lock().await;
        let tmp = tempfile::tempdir().map_err(storage)?;
        let db_path = tmp.path().join("memories.sqlite");
        // Esquema idéntico al actual, vacío.
        drop(Database::open(&db_path).map_err(storage)?);

        {
            let conn = self.database().conn();
            conn.execute("ATTACH DATABASE ?1 AS exp", [db_path.to_string_lossy()]).map_err(storage)?;
            let result = (|| -> MemoryResult<()> {
                conn.execute_batch("BEGIN").map_err(storage)?;
                let kinds: Vec<&str> = options.kinds.as_ref().map_or_else(
                    || MemoryKind::ALL.iter().map(|k| k.as_str()).collect(),
                    |k| k.iter().map(|k| k.as_str()).collect(),
                );
                let list = kinds.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(",");
                conn.execute(&format!("INSERT INTO exp.memories SELECT * FROM main.memories WHERE kind IN ({list})"), []).map_err(storage)?;
                conn.execute(
                    "INSERT INTO exp.memory_sources SELECT * FROM main.memory_sources WHERE id IN (SELECT source_id FROM exp.memories)",
                    [],
                )
                .map_err(storage)?;
                conn.execute(
                    "INSERT INTO exp.memory_relations SELECT * FROM main.memory_relations
                     WHERE source_id IN (SELECT id FROM exp.memories) AND target_id IN (SELECT id FROM exp.memories)",
                    [],
                )
                .map_err(storage)?;
                conn.execute(
                    "INSERT INTO exp.embedding_metadata SELECT * FROM main.embedding_metadata WHERE memory_id IN (SELECT id FROM exp.memories)",
                    [],
                )
                .map_err(storage)?;
                if options.include_experiences {
                    conn.execute("INSERT INTO exp.experiences SELECT * FROM main.experiences", []).map_err(storage)?;
                }
                if options.include_skills {
                    conn.execute("INSERT INTO exp.skills SELECT * FROM main.skills", []).map_err(storage)?;
                    conn.execute("INSERT INTO exp.skill_versions SELECT * FROM main.skill_versions", []).map_err(storage)?;
                }
                conn.execute_batch("COMMIT").map_err(storage)
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            let _ = conn.execute("DETACH DATABASE exp", []);
            result?;
        }

        // Limpieza defensiva y compactación del archivo exportado.
        let (counts, skills_files) = {
            let conn = Connection::open(&db_path).map_err(storage)?;
            let rows: Vec<(String, String)> = {
                let mut stmt = conn.prepare("SELECT id, data FROM experiences").map_err(storage)?;
                let r = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(storage)?;
                r.collect::<Result<_, _>>().map_err(storage)?
            };
            for (id, data) in rows {
                conn.execute("UPDATE experiences SET data = ?2 WHERE id = ?1", params![id, redact_secrets(&data)]).map_err(storage)?;
            }
            let skills: Vec<Skill> = {
                let mut stmt = conn
                    .prepare("SELECT id, name, description, procedure, enabled, requires_review, source, experience_id, current_version, created_at, updated_at FROM skills")
                    .map_err(storage)?;
                let r = stmt.query_map([], super::procedural_memory::row_to_skill).map_err(storage)?;
                r.collect::<Result<_, _>>().map_err(storage)?
            };
            let mut files = Vec::new();
            for s in skills {
                let versions = {
                    let mut stmt = conn
                        .prepare("SELECT skill_id, version, procedure, notes, created_at FROM skill_versions WHERE skill_id = ?1 ORDER BY version")
                        .map_err(storage)?;
                    let r = stmt
                        .query_map([&s.id], |r| {
                            let created: String = r.get(4)?;
                            Ok(SkillVersion {
                                skill_id: r.get(0)?,
                                version: r.get(1)?,
                                procedure: r.get(2)?,
                                notes: r.get(3)?,
                                created_at: chrono::DateTime::parse_from_rfc3339(&created).map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc)),
                            })
                        })
                        .map_err(storage)?;
                    r.collect::<Result<Vec<_>, _>>().map_err(storage)?
                };
                files.push(SkillFile { skill: s, versions });
            }
            let counts = Counts {
                memories: count(&conn, "memories")?,
                relations: count(&conn, "memory_relations")?,
                sources: count(&conn, "memory_sources")?,
                embeddings: count(&conn, "embedding_metadata")?,
                experiences: count(&conn, "experiences")?,
                skills: count(&conn, "skills")?,
            };
            conn.execute_batch("VACUUM").map_err(storage)?;
            (counts, files)
        };

        let mut files: Vec<(String, Vec<u8>)> = vec![("memories.sqlite".into(), std::fs::read(&db_path).map_err(storage)?)];
        for f in &skills_files {
            files.push((format!("skills/{}.json", f.skill.id), serde_json::to_vec_pretty(f).map_err(storage)?));
        }
        let checksums: BTreeMap<String, String> = files.iter().map(|(n, d)| (n.clone(), sha256(d))).collect();
        let manifest = Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            exported_at: Utc::now().to_rfc3339(),
            installation_id: self.repo.installation_id()?,
            embedding_model: self.embedder().model_id(),
            counts,
            skills: skills_files.iter().map(|f| SkillRef { id: f.skill.id.clone(), name: f.skill.name.clone(), version: f.skill.current_version }).collect(),
            checksums: checksums.clone(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(storage)?;
        let mut all_sums = checksums;
        all_sums.insert("manifest.json".into(), sha256(&manifest_bytes));
        files.push(("manifest.json".into(), manifest_bytes));
        files.push(("checksums.json".into(), serde_json::to_vec_pretty(&all_sums).map_err(storage)?));

        let archive = tar(&files, &["skills", "attachments"])?;
        let sealed = seal(&archive, passphrase, kdf)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(storage)?;
        }
        std::fs::write(path, &sealed).map_err(storage)?;
        self.repo.log_event("MEMORY_EXPORTED", None, &[], Some(&path.display().to_string()))?;
        Ok(ExportReport { path: path.display().to_string(), bytes: sealed.len() as u64, manifest })
    }

    /// Copia de seguridad completa de la base (`VACUUM INTO`).
    ///
    /// # Errors
    /// Errores de E/S o de SQLite.
    pub fn backup(&self, label: &str) -> MemoryResult<PathBuf> {
        std::fs::create_dir_all(&self.config.backups_dir).map_err(storage)?;
        let safe: String = label.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let path = self.config.backups_dir.join(format!("jarvis-{}-{}.db", safe, Utc::now().format("%Y%m%d-%H%M%S%3f")));
        self.database().conn().execute("VACUUM INTO ?1", [path.to_string_lossy()]).map_err(storage)?;
        Ok(path)
    }

    /// # Errors
    /// Errores de E/S.
    pub fn list_backups(&self) -> MemoryResult<Vec<BackupInfo>> {
        let Ok(dir) = std::fs::read_dir(&self.config.backups_dir) else { return Ok(Vec::new()) };
        let mut out: Vec<BackupInfo> = dir
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("db"))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some(BackupInfo {
                    name: e.file_name().to_string_lossy().into_owned(),
                    bytes: meta.len(),
                    modified: meta.modified().ok().map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339()).unwrap_or_default(),
                })
            })
            .collect();
        out.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(out)
    }

    /// Restaura las tablas de memoria desde un respaldo (antes guarda otro del estado actual).
    ///
    /// # Errors
    /// Nombre inválido, respaldo de otra versión del esquema o errores de SQLite.
    pub fn restore_backup(&self, name: &str) -> MemoryResult<PathBuf> {
        if name.contains(['/', '\\']) || !name.ends_with(".db") || name.contains("..") {
            return Err(MemoryError::Invalid("nombre de respaldo inválido".into()));
        }
        let path = self.config.backups_dir.join(name);
        if !path.is_file() {
            return Err(MemoryError::NotFound);
        }
        let safety = self.backup("pre-restore")?;
        let conn = self.database().conn();
        conn.execute("ATTACH DATABASE ?1 AS bk", [path.to_string_lossy()]).map_err(storage)?;
        let result = (|| -> MemoryResult<()> {
            let v: i64 = conn.query_row("PRAGMA bk.user_version", [], |r| r.get(0)).map_err(storage)?;
            if v != SCHEMA_VERSION {
                return Err(MemoryError::Invalid(format!("el respaldo es del esquema v{v} y la app usa v{SCHEMA_VERSION}")));
            }
            conn.execute_batch("BEGIN").map_err(storage)?;
            for t in MEMORY_TABLES_DELETE_ORDER {
                conn.execute(&format!("DELETE FROM main.{t}"), []).map_err(storage)?;
            }
            for t in MEMORY_TABLES_DELETE_ORDER.iter().rev() {
                conn.execute(&format!("INSERT INTO main.{t} SELECT * FROM bk.{t}"), []).map_err(storage)?;
            }
            conn.execute_batch("COMMIT").map_err(storage)
        })();
        if result.is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        let _ = conn.execute("DETACH DATABASE bk", []);
        drop(conn);
        result?;
        self.invalidate_cache();
        self.repo.log_event("MEMORY_RESTORED", None, &[], Some(name))?;
        self.emit(crate::domain::VisualEvent::MemoryImported { imported: 0, restored: true });
        Ok(safety)
    }
}

/// Tablas de memoria, de hijas a padres (orden de borrado).
pub(crate) const MEMORY_TABLES_DELETE_ORDER: &[&str] =
    &["embedding_metadata", "memory_relations", "memory_events", "skill_versions", "skills", "experiences", "memories", "memory_sources"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub bytes: u64,
    pub modified: String,
}
