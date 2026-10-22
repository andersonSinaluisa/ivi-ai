//! Importación de un paquete `.jarvismem`.
//!
//! 1. Descifra y extrae (rutas validadas).
//! 2. Comprueba los checksums y la compatibilidad (formato y esquema).
//! 3. Abre la base importada, que se migra si es de una versión anterior.
//! 4. Vista previa: nuevos, duplicados (mismo contenido) y conflictos (mismo id, distinto contenido).
//! 5. Aplicación selectiva con estrategia de conflictos, previa copia de seguridad.
//! 6. Relaciones reasignadas a los ids finales; las inferidas nunca llegan verificadas.
//! 7. Embeddings copiados si el modelo coincide; si no, se recalculan.
//! 8. Habilidades importadas desactivadas y pendientes de revisión: nunca se ejecutan solas.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::memory_engine::MemoryEngine;
use super::memory_exporter::{sha256, Manifest, FORMAT, FORMAT_VERSION};
use super::memory_package::{check_extension, open, untar, MAX_PACKAGE_BYTES};
use super::memory_repository::{storage, MemoryRepository};
use super::{MemoryError, MemoryResult};
use crate::domain::{MemoryKind, MemoryRecord, NewSource, Provenance, SourceKind, VisualEvent};
use crate::infrastructure::persistence::{Database, SCHEMA_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    /// Conservar el recuerdo local y descartar el importado.
    #[default]
    KeepLocal,
    /// Sobrescribir el local con el importado.
    KeepImported,
    /// Conservar ambos (el importado recibe un id nuevo).
    KeepBoth,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportSelection {
    pub kinds: Option<Vec<MemoryKind>>,
    /// Importación selectiva por id (None = todos los del tipo elegido).
    pub memory_ids: Option<Vec<String>>,
    pub include_relations: bool,
    pub include_experiences: bool,
    pub include_skills: bool,
    pub conflict: ConflictStrategy,
}

impl Default for ImportSelection {
    fn default() -> Self {
        Self {
            kinds: None,
            memory_ids: None,
            include_relations: true,
            include_experiences: true,
            include_skills: true,
            conflict: ConflictStrategy::KeepLocal,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    pub id: String,
    pub imported_summary: String,
    pub local_summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMemory {
    pub id: String,
    pub kind: MemoryKind,
    pub summary: String,
    /// new | duplicate | conflict
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub manifest: Manifest,
    pub same_installation: bool,
    pub new_memories: usize,
    pub duplicates: usize,
    pub conflicts: Vec<ConflictItem>,
    pub needs_reembed: bool,
    pub memories: Vec<PreviewMemory>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub duplicates_skipped: usize,
    pub conflicts_kept_local: usize,
    pub conflicts_overwritten: usize,
    pub conflicts_duplicated: usize,
    pub relations: usize,
    pub embeddings_copied: usize,
    pub reembedded: usize,
    pub experiences: usize,
    pub skills_pending_review: usize,
    pub backup: String,
}

/// Paquete descifrado y verificado, listo para leer.
struct Unpacked {
    _dir: tempfile::TempDir,
    manifest: Manifest,
    db: Database,
}

fn package_err(msg: impl Into<String>) -> MemoryError {
    MemoryError::Package(msg.into())
}

fn unpack(path: &Path, passphrase: &str) -> MemoryResult<Unpacked> {
    check_extension(path)?;
    let meta = std::fs::metadata(path).map_err(|e| package_err(format!("no se puede leer {}: {e}", path.display())))?;
    if meta.len() as usize > MAX_PACKAGE_BYTES {
        return Err(package_err("el archivo es demasiado grande"));
    }
    let sealed = std::fs::read(path).map_err(storage)?;
    let files: BTreeMap<String, Vec<u8>> = untar(&open(&sealed, passphrase)?)?.into_iter().collect();

    let sums_raw = files.get("checksums.json").ok_or_else(|| package_err("falta checksums.json"))?;
    let sums: BTreeMap<String, String> = serde_json::from_slice(sums_raw).map_err(|e| package_err(format!("checksums.json inválido: {e}")))?;
    for (name, data) in files.iter().filter(|(n, _)| n.as_str() != "checksums.json") {
        match sums.get(name) {
            Some(expected) if *expected == sha256(data) => {}
            Some(_) => return Err(package_err(format!("checksum incorrecto: {name}"))),
            None => return Err(package_err(format!("archivo no declarado en checksums.json: {name}"))),
        }
    }
    let manifest: Manifest = serde_json::from_slice(files.get("manifest.json").ok_or_else(|| package_err("falta manifest.json"))?)
        .map_err(|e| package_err(format!("manifest.json inválido: {e}")))?;
    for (name, expected) in &manifest.checksums {
        let actual = files.get(name).map(|d| sha256(d));
        if actual.as_deref() != Some(expected.as_str()) {
            return Err(package_err(format!("el manifiesto no coincide con {name}")));
        }
    }
    if manifest.format != FORMAT || manifest.format_version != FORMAT_VERSION {
        return Err(package_err(format!("formato no soportado: {} v{}", manifest.format, manifest.format_version)));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(package_err(format!(
            "el paquete usa el esquema v{} y esta versión de Jarvis solo conoce hasta la v{SCHEMA_VERSION}: actualiza Jarvis",
            manifest.schema_version
        )));
    }
    let dir = tempfile::tempdir().map_err(storage)?;
    let db_path = dir.path().join("memories.sqlite");
    std::fs::write(&db_path, files.get("memories.sqlite").ok_or_else(|| package_err("falta memories.sqlite"))?).map_err(storage)?;
    // Abrirla aplica las migraciones pendientes si viene de un esquema anterior.
    let db = Database::open(&db_path).map_err(|e| package_err(format!("memories.sqlite inválido: {e}")))?;
    Ok(Unpacked { _dir: dir, manifest, db })
}

fn imported_memories(repo: &MemoryRepository) -> MemoryResult<Vec<MemoryRecord>> {
    repo.recent(usize::MAX >> 1, 0)
}

impl MemoryEngine {
    /// # Errors
    /// Contraseña incorrecta, paquete dañado o incompatible.
    pub fn preview_import(&self, path: &Path, passphrase: &str) -> MemoryResult<ImportPreview> {
        let u = unpack(path, passphrase)?;
        let imported = MemoryRepository::new(u.db.clone())?;
        let mut preview = ImportPreview {
            same_installation: u.manifest.installation_id == self.repo.installation_id()?,
            needs_reembed: u.manifest.embedding_model != self.embedder().model_id(),
            manifest: u.manifest,
            new_memories: 0,
            duplicates: 0,
            conflicts: Vec::new(),
            memories: Vec::new(),
        };
        for m in imported_memories(&imported)? {
            let status = match self.repo.get(&m.id)? {
                Some(local) if local.content_hash == m.content_hash => "duplicate",
                Some(local) => {
                    preview.conflicts.push(ConflictItem { id: m.id.clone(), imported_summary: m.label(), local_summary: local.label() });
                    "conflict"
                }
                None if self.repo.find_by_hash(&m.content_hash)?.is_some() => "duplicate",
                None => "new",
            };
            match status {
                "new" => preview.new_memories += 1,
                "duplicate" => preview.duplicates += 1,
                _ => {}
            }
            if preview.memories.len() < 2_000 {
                preview.memories.push(PreviewMemory { id: m.id.clone(), kind: m.kind, summary: m.label(), status });
            }
        }
        Ok(preview)
    }

    /// # Errors
    /// Paquete inválido o errores de SQLite. Ante un error a medias queda la copia de seguridad.
    pub async fn apply_import(&self, path: &Path, passphrase: &str, selection: &ImportSelection) -> MemoryResult<ImportReport> {
        let u = unpack(path, passphrase)?;
        let backup = self.backup("pre-import")?;
        let mut report = ImportReport { backup: backup.display().to_string(), ..ImportReport::default() };
        let imported = MemoryRepository::new(u.db.clone())?;
        let source = self.repo.insert_source(&NewSource {
            kind: SourceKind::Import,
            reference: Some(u.manifest.installation_id.clone()),
            label: Some(format!("importado ({})", &u.manifest.exported_at[..10.min(u.manifest.exported_at.len())])),
        })?;

        let wanted_ids: Option<HashSet<&String>> = selection.memory_ids.as_ref().map(|v| v.iter().collect());
        let mut id_map: HashMap<String, String> = HashMap::new();
        let mut inserted: Vec<(String, String)> = Vec::new(); // (id final, id original)
        for mut m in imported_memories(&imported)? {
            if selection.kinds.as_ref().is_some_and(|k| !k.contains(&m.kind)) || wanted_ids.as_ref().is_some_and(|w| !w.contains(&m.id)) {
                continue;
            }
            let original = m.id.clone();
            if let Some(local) = self.repo.get(&m.id)? {
                if local.content_hash == m.content_hash {
                    report.duplicates_skipped += 1;
                    id_map.insert(original, local.id);
                    continue;
                }
                match selection.conflict {
                    ConflictStrategy::KeepLocal => {
                        report.conflicts_kept_local += 1;
                        id_map.insert(original, local.id);
                        continue;
                    }
                    ConflictStrategy::KeepImported => {
                        m.updated_at = Utc::now();
                        self.repo.update(&m)?;
                        self.cache_remove(&m.id);
                        report.conflicts_overwritten += 1;
                        inserted.push((m.id.clone(), original.clone()));
                        id_map.insert(original, m.id);
                        continue;
                    }
                    ConflictStrategy::KeepBoth => {
                        m.id = uuid::Uuid::new_v4().to_string();
                        report.conflicts_duplicated += 1;
                    }
                }
            } else if let Some(local) = self.repo.find_by_hash(&m.content_hash)? {
                report.duplicates_skipped += 1;
                id_map.insert(original, local.id);
                continue;
            }
            m.source = source.label.clone();
            m.source_id = Some(source.id.clone());
            self.repo.insert(&m)?;
            report.imported += 1;
            inserted.push((m.id.clone(), original.clone()));
            id_map.insert(original, m.id);
        }

        if selection.include_relations {
            for mut r in imported.all_relations()? {
                let (Some(src), Some(tgt)) = (id_map.get(&r.source_id), id_map.get(&r.target_id)) else { continue };
                if src == tgt {
                    continue;
                }
                r.source_id = src.clone();
                r.target_id = tgt.clone();
                if self.repo.get_relation(&r.id)?.is_some() {
                    r.id = uuid::Uuid::new_v4().to_string();
                }
                // Lo inferido no se convierte en verificado por pasar por un archivo.
                if r.provenance == Provenance::Inferred {
                    r.verified = false;
                }
                if self.repo.insert_relation(&r)? {
                    report.relations += 1;
                }
            }
        }

        // Embeddings: se reutilizan si el modelo coincide.
        let model = self.embedder().model_id();
        if u.manifest.embedding_model == model {
            let vectors: HashMap<String, (String, Vec<f32>)> =
                imported.embeddings_for_model(&model)?.into_iter().map(|(id, h, v)| (id, (h, v))).collect();
            for (final_id, original) in &inserted {
                if let Some((hash, v)) = vectors.get(original) {
                    self.repo.upsert_embedding(final_id, &model, hash, v)?;
                    report.embeddings_copied += 1;
                }
            }
        }

        if selection.include_experiences {
            report.experiences = self.copy_experiences(&u.db)?;
        }
        if selection.include_skills {
            report.skills_pending_review = self.copy_skills(&u.db)?;
        }

        self.invalidate_cache();
        // Lo que no tiene vector vigente (modelo distinto o paquete sin vectores) se recalcula.
        report.reembedded = match self.reembed(&CancellationToken::new()).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "embeddings pendientes tras importar");
                0
            }
        };
        let ids: Vec<String> = inserted.iter().map(|(id, _)| id.clone()).collect();
        self.repo.log_event("MEMORY_IMPORTED", None, &ids, Some(&path.display().to_string()))?;
        self.emit(VisualEvent::MemoryImported { imported: report.imported, restored: false });
        Ok(report)
    }

    fn copy_experiences(&self, other: &Database) -> MemoryResult<usize> {
        type Row = (String, String, i64, i64, String, String, i64, String);
        let rows: Vec<Row> = {
            let conn = other.conn();
            let mut stmt = conn
                .prepare("SELECT id, data, verified, promoted, created_at, signature, occurrences, updated_at FROM experiences")
                .map_err(storage)?;
            let r = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))
                .map_err(storage)?;
            r.collect::<Result<_, _>>().map_err(storage)?
        };
        let conn = self.database().conn();
        let mut n = 0;
        for (id, data, verified, promoted, created, signature, occ, updated) in rows {
            let exists: Option<i64> = conn
                .query_row("SELECT 1 FROM experiences WHERE id = ?1 OR (signature <> '' AND signature = ?2)", params![id, signature], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            if exists.is_some() {
                continue;
            }
            conn.execute(
                "INSERT INTO experiences (id, data, verified, promoted, created_at, signature, occurrences, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, data, verified, promoted, created, signature, occ, updated],
            )
            .map_err(storage)?;
            n += 1;
        }
        Ok(n)
    }

    fn copy_skills(&self, other: &Database) -> MemoryResult<usize> {
        let path = other.path().ok_or_else(|| package_err("base importada sin archivo"))?.to_path_buf();
        let conn = self.database().conn();
        conn.execute("ATTACH DATABASE ?1 AS imp", [path.to_string_lossy()]).map_err(storage)?;
        let result = (|| -> MemoryResult<usize> {
            // Desactivadas y pendientes de revisión: el usuario decide si las activa.
            let n = conn
                .execute(
                    "INSERT INTO main.skills (id, name, description, procedure, enabled, requires_review, source, experience_id, current_version, created_at, updated_at)
                     SELECT id, name, description, procedure, 0, 1, 'import', experience_id, current_version, created_at, updated_at
                     FROM imp.skills WHERE id NOT IN (SELECT id FROM main.skills)",
                    [],
                )
                .map_err(storage)?;
            conn.execute("INSERT OR IGNORE INTO main.skill_versions SELECT * FROM imp.skill_versions", []).map_err(storage)?;
            Ok(n)
        })();
        let _ = conn.execute("DETACH DATABASE imp", []);
        result
    }
}

/// Lectura rápida del manifiesto sin importar nada (para listar paquetes).
///
/// # Errors
/// Contraseña incorrecta o paquete dañado.
pub fn read_manifest(path: &Path, passphrase: &str) -> MemoryResult<Manifest> {
    unpack(path, passphrase).map(|u| u.manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MemoryPatch, NewMemory, RelationKind};
    use rusqlite::Connection;
    use crate::memory::memory_exporter::ExportOptions;
    use crate::memory::memory_package::KdfParams;
    use crate::memory::experience_memory::tests::experience;

    const PASS: &str = "clave de prueba segura";

    async fn engine_with_data() -> (MemoryEngine, Vec<String>) {
        let e = MemoryEngine::for_tests().expect("e");
        let a = e.remember(NewMemory::user(MemoryKind::Entity, "LOOP: SaaS de cupones")).await.expect("a").memory.id;
        let b = e.remember(NewMemory::user(MemoryKind::Semantic, "LOOP usa FastAPI y PostgreSQL")).await.expect("b").memory.id;
        e.relate(&b, &a, RelationKind::BelongsTo, Provenance::User, None).expect("rel");
        e.save_experience_dedup(&experience("x1", "saber la hora", true)).expect("x");
        e.promote_experience("x1").await.expect("skill");
        (e, vec![a, b])
    }

    fn file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("mem.jarvismem")
    }

    #[tokio::test]
    async fn export_then_import_into_a_fresh_installation() {
        let (src, ids) = engine_with_data().await;
        let dir = tempfile::tempdir().expect("tmp");
        let report = src.export_memory(&file(&dir), PASS, &ExportOptions::default(), KdfParams::fast()).await.expect("export");
        assert_eq!(report.manifest.counts.memories, 3, "2 recuerdos + el procedimiento de la habilidad");
        assert_eq!(report.manifest.counts.skills, 1);
        assert_eq!(report.manifest.schema_version, SCHEMA_VERSION);
        assert!(report.manifest.checksums.contains_key("memories.sqlite"));

        let dst = MemoryEngine::for_tests().expect("dst");
        let preview = dst.preview_import(&file(&dir), PASS).expect("preview");
        assert!(!preview.same_installation);
        assert_eq!(preview.new_memories, 3);
        assert!(!preview.needs_reembed);

        let r = dst.apply_import(&file(&dir), PASS, &ImportSelection::default()).await.expect("import");
        assert_eq!(r.imported, 3);
        assert_eq!(r.relations, src.repository().all_relations().expect("r").len());
        assert_eq!(r.embeddings_copied, 3);
        assert_eq!(r.reembedded, 0);
        assert!(std::path::Path::new(&r.backup).is_file(), "se hizo una copia antes de importar");
        // Mismos UUID y relaciones.
        let imported = dst.repository().get(&ids[1]).expect("g").expect("existe con el mismo id");
        assert_eq!(imported.content, "LOOP usa FastAPI y PostgreSQL");
        assert_eq!(imported.source.split(' ').next(), Some("importado"));
        assert!(dst.repository().relations_of(&ids[1]).expect("r").iter().any(|r| r.kind == RelationKind::BelongsTo));
        // Habilidad importada: desactivada y pendiente de revisión.
        let skills = dst.skills().expect("s");
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].enabled && skills[0].requires_review);
        assert_eq!(r.experiences, 1);

        // Reimportar: todo duplicado, nada nuevo.
        let again = dst.apply_import(&file(&dir), PASS, &ImportSelection::default()).await.expect("again");
        assert_eq!(again.imported, 0);
        assert_eq!(again.duplicates_skipped, 3);
    }

    #[tokio::test]
    async fn conflicts_follow_the_chosen_strategy() {
        let (src, ids) = engine_with_data().await;
        let dir = tempfile::tempdir().expect("tmp");
        src.export_memory(&file(&dir), PASS, &ExportOptions::default(), KdfParams::fast()).await.expect("export");
        // Mismo id, contenido distinto en el destino (misma instalación editada después).
        src.update_memory(&ids[1], MemoryPatch { content: Some("LOOP usa Django".into()), ..MemoryPatch::default() }).await.expect("edit");

        let preview = src.preview_import(&file(&dir), PASS).expect("preview");
        assert!(preview.same_installation);
        assert_eq!(preview.conflicts.len(), 1);

        let keep_local = src.apply_import(&file(&dir), PASS, &ImportSelection::default()).await.expect("local");
        assert_eq!(keep_local.conflicts_kept_local, 1);
        assert_eq!(src.repository().get(&ids[1]).expect("g").expect("m").content, "LOOP usa Django");

        let both = src
            .apply_import(&file(&dir), PASS, &ImportSelection { conflict: ConflictStrategy::KeepBoth, ..ImportSelection::default() })
            .await
            .expect("both");
        assert_eq!(both.conflicts_duplicated, 1);
        assert_eq!(src.repository().count().expect("n"), 4);

        // Tras KeepBoth, el contenido importado ya existe como duplicado: KeepImported sobrescribe el original.
        let imported = src
            .apply_import(&file(&dir), PASS, &ImportSelection { conflict: ConflictStrategy::KeepImported, ..ImportSelection::default() })
            .await
            .expect("imported");
        assert_eq!(imported.conflicts_overwritten, 1);
        assert_eq!(src.repository().get(&ids[1]).expect("g").expect("m").content, "LOOP usa FastAPI y PostgreSQL");
    }

    #[tokio::test]
    async fn selective_import_and_model_change_trigger_reembedding() {
        let (src, ids) = engine_with_data().await;
        let dir = tempfile::tempdir().expect("tmp");
        src.export_memory(&file(&dir), PASS, &ExportOptions::default(), KdfParams::fast()).await.expect("export");

        let dst = MemoryEngine::for_tests().expect("dst");
        // El destino usa otro modelo de embeddings.
        dst.set_embedder(std::sync::Arc::new(crate::memory::embedding_provider::HashingEmbedder::new(128)), "otro".into());
        let preview = dst.preview_import(&file(&dir), PASS).expect("preview");
        assert!(preview.needs_reembed);
        let sel = ImportSelection { memory_ids: Some(vec![ids[0].clone()]), include_skills: false, include_experiences: false, ..ImportSelection::default() };
        let r = dst.apply_import(&file(&dir), PASS, &sel).await.expect("import");
        assert_eq!(r.imported, 1);
        assert_eq!(r.embeddings_copied, 0);
        assert_eq!(r.reembedded, 1, "vectores recalculados con el modelo del destino");
        assert_eq!(dst.embedding_status().expect("s").pending, 0);
        assert!(dst.skills().expect("s").is_empty());
    }

    #[tokio::test]
    async fn bad_passwords_tampering_and_newer_schemas_are_rejected() {
        let (src, _) = engine_with_data().await;
        let dir = tempfile::tempdir().expect("tmp");
        src.export_memory(&file(&dir), PASS, &ExportOptions::default(), KdfParams::fast()).await.expect("export");
        let dst = MemoryEngine::for_tests().expect("dst");
        assert!(dst.preview_import(&file(&dir), "clave equivocada").is_err());
        let mut bytes = std::fs::read(file(&dir)).expect("read");
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        let tampered = dir.path().join("t.jarvismem");
        std::fs::write(&tampered, &bytes).expect("w");
        assert!(dst.preview_import(&tampered, PASS).is_err());
        assert!(src.export_memory(&dir.path().join("x.zip"), PASS, &ExportOptions::default(), KdfParams::fast()).await.is_err());

        // Paquete válido pero de un esquema futuro.
        let mut files: Vec<(String, Vec<u8>)> = untar(&open(&std::fs::read(file(&dir)).expect("r"), PASS).expect("o")).expect("u");
        let mut manifest: Manifest = serde_json::from_slice(&files.iter().find(|(n, _)| n == "manifest.json").expect("m").1).expect("json");
        manifest.schema_version = SCHEMA_VERSION + 1;
        let mbytes = serde_json::to_vec(&manifest).expect("j");
        let mut sums: BTreeMap<String, String> = serde_json::from_slice(&files.iter().find(|(n, _)| n == "checksums.json").expect("c").1).expect("j");
        sums.insert("manifest.json".into(), sha256(&mbytes));
        for f in &mut files {
            if f.0 == "manifest.json" {
                f.1 = mbytes.clone();
            } else if f.0 == "checksums.json" {
                f.1 = serde_json::to_vec(&sums).expect("j");
            }
        }
        let sealed = crate::memory::memory_package::seal(&crate::memory::memory_package::tar(&files, &[]).expect("t"), PASS, KdfParams::fast()).expect("s");
        let future = dir.path().join("future.jarvismem");
        std::fs::write(&future, sealed).expect("w");
        let e = dst.preview_import(&future, PASS).expect_err("futuro");
        assert!(e.to_string().contains("actualiza Jarvis"), "{e}");
    }

    #[tokio::test]
    async fn exports_contain_no_conversations_or_secrets() {
        let (src, _) = engine_with_data().await;
        let conv = src.database().ensure_conversation(None, "hola").expect("c");
        src.database().append_message(&conv, &crate::domain::ChatMessage::user("mensaje privado de la conversación")).expect("m");
        let mut exp = experience("x2", "otra tarea", true);
        exp.result = "mi password: supersecreta".into();
        src.database()
            .conn()
            .execute("INSERT INTO experiences (id, data, verified, promoted, created_at) VALUES ('x2', ?1, 1, 0, ?2)", params![serde_json::to_string(&exp).expect("j"), Utc::now().to_rfc3339()])
            .expect("raw");
        let dir = tempfile::tempdir().expect("tmp");
        src.export_memory(&file(&dir), PASS, &ExportOptions::default(), KdfParams::fast()).await.expect("export");
        let files = untar(&open(&std::fs::read(file(&dir)).expect("r"), PASS).expect("o")).expect("u");
        let sqlite = &files.iter().find(|(n, _)| n == "memories.sqlite").expect("db").1;
        let path = dir.path().join("check.sqlite");
        std::fs::write(&path, sqlite).expect("w");
        let conn = Connection::open(&path).expect("open");
        let messages: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).expect("n");
        assert_eq!(messages, 0, "las conversaciones no se exportan");
        let leaked: i64 = conn.query_row("SELECT COUNT(*) FROM experiences WHERE data LIKE '%supersecreta%'", [], |r| r.get(0)).expect("n");
        assert_eq!(leaked, 0);
        let meta: i64 = conn.query_row("SELECT COUNT(*) FROM app_meta", [], |r| r.get(0)).expect("n");
        assert_eq!(meta, 0);
    }

    #[tokio::test]
    async fn backups_can_be_listed_and_restored() {
        let (e, ids) = engine_with_data().await;
        e.backup("manual").expect("b");
        e.delete_memory(&ids[0]).expect("del");
        assert_eq!(e.repository().count().expect("n"), 2);
        let name = e.list_backups().expect("l").into_iter().find(|b| b.name.contains("manual")).expect("backup").name;
        let safety = e.restore_backup(&name).expect("restore");
        assert!(safety.is_file());
        assert_eq!(e.repository().count().expect("n"), 3);
        assert!(e.repository().get(&ids[0]).expect("g").is_some());
        assert!(matches!(e.restore_backup("../x.db"), Err(MemoryError::Invalid(_))));
    }
}
