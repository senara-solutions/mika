//! # Lexical Graph Ingestor
//!
//! Per-agent ingestion of `docs/solutions/**/*.md` into the lexical layer of
//! the Knowledge Graph (#689). Each document is:
//!
//! 1. Read and normalized (BOM strip, LF line endings, trailing-ws strip).
//! 2. Content-hashed (SHA-256) for idempotent skip-on-unchanged.
//! 3. Chunked via [`super::chunker::chunk_doc`].
//! 4. Written to `kg_chunks` + `search_content` + `fts_search` in a single
//!    transaction (per conventions C1.1 — embeddings backfilled async).
//!
//! Documents removed from disk are pruned (symmetric delete via
//! [`Database::delete_search_content`]).
//!
//! ## Sole-Writer Contract
//!
//! This module is the sole writer of `kg_chunks` rows and of
//! `source_type="kg_chunk"` entries in `search_content`. No other code path
//! should write these.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::kg_schema::KG_CHUNK_SOURCE_TYPE;

use super::chunker::chunk_doc;

/// Statistics returned from a full ingestion run.
#[derive(Debug, Clone)]
pub struct IngestStats {
    pub docs_scanned: usize,
    pub docs_skipped_unchanged: usize,
    pub docs_ingested: usize,
    pub docs_pruned: usize,
    pub chunks_added: usize,
    pub chunks_deleted: usize,
    pub duration_ms: u64,
}

/// Statistics returned from a single-doc ingestion.
#[derive(Debug, Clone)]
pub struct DocStats {
    pub outcome: DocOutcome,
    pub chunks_added: usize,
    pub chunks_deleted: usize,
}

/// Outcome of ingesting a single document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocOutcome {
    /// Doc was unchanged (content hash matched) — skipped.
    Skipped,
    /// Doc was new or changed — (re)ingested.
    Ingested,
    /// Doc was pruned (no longer on disk).
    Pruned,
}

/// Session ID sentinel for audit events emitted during startup ingestion.
const STARTUP_SESSION_ID: &str = "startup";

/// Per-agent lexical graph ingestor.
///
/// Scans a docs directory, chunks markdown files, and writes them into
/// `kg_chunks` + `search_content` with content-hash idempotency.
pub struct LexicalIngestor {
    db: AsyncDatabase,
    docs_root: PathBuf,
    trace_id: String,
    /// Session ID for audit events. Defaults to [`STARTUP_SESSION_ID`] for
    /// startup scans; compound hook callers may pass the active session.
    session_id: String,
}

impl LexicalIngestor {
    /// Create a new ingestor.
    ///
    /// - `db`: async database handle (carries `agent_id` implicitly).
    /// - `docs_root`: root directory to scan for `*.md` files.
    /// - `trace_id`: optional trace ID for observability. If `None`, a fresh
    ///   UUID is generated.
    pub fn new(db: AsyncDatabase, docs_root: PathBuf, trace_id: Option<&str>) -> Self {
        let trace_id = trace_id
            .map(|s| s.to_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));
        Self {
            db,
            docs_root,
            trace_id,
            session_id: STARTUP_SESSION_ID.to_owned(),
        }
    }

    /// Create an ingestor with a specific session ID (for compound hook use).
    pub fn with_session(mut self, session_id: &str) -> Self {
        self.session_id = session_id.to_owned();
        self
    }

    /// Ingest all `*.md` files under `docs_root` for the agent.
    ///
    /// Returns statistics about the run. Failures on individual documents are
    /// logged and skipped — the run continues.
    pub async fn ingest_all(&self) -> Result<IngestStats> {
        let start = Instant::now();
        let agent_id = self.db.agent_id.clone();

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            docs_root = %self.docs_root.display(),
            event = "lexical_ingest_start",
        );

        // 1. Walk docs_root for *.md files.
        let on_disk = self.scan_docs()?;

        let mut stats = IngestStats {
            docs_scanned: on_disk.len(),
            docs_skipped_unchanged: 0,
            docs_ingested: 0,
            docs_pruned: 0,
            chunks_added: 0,
            chunks_deleted: 0,
            duration_ms: 0,
        };

        // 2. For each file: read, normalize, hash, compare, ingest if changed.
        for path in &on_disk {
            let rel_path = self
                .relative_path(path)
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            match self.ingest_single_doc_inner(path).await {
                Ok(doc_stats) => {
                    // Emit per-document audit event (C3.2).
                    self.emit_audit_event(&rel_path, &doc_stats).await;

                    match doc_stats.outcome {
                        DocOutcome::Skipped => stats.docs_skipped_unchanged += 1,
                        DocOutcome::Ingested => {
                            stats.docs_ingested += 1;
                            stats.chunks_added += doc_stats.chunks_added;
                            stats.chunks_deleted += doc_stats.chunks_deleted;
                        }
                        DocOutcome::Pruned => {
                            stats.docs_pruned += 1;
                            stats.chunks_deleted += doc_stats.chunks_deleted;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %path.display(),
                        error = %e,
                        event = "lexical_ingest_doc_failed",
                    );
                }
            }
        }

        // 3. Prune removed docs.
        let on_disk_paths: HashSet<String> = on_disk
            .iter()
            .filter_map(|p| self.relative_path(p))
            .collect();

        match self.prune_removed_docs(&on_disk_paths).await {
            Ok((pruned_count, deleted_chunks)) => {
                stats.docs_pruned += pruned_count;
                stats.chunks_deleted += deleted_chunks;
            }
            Err(e) => {
                warn!(
                    trace_id = %self.trace_id,
                    agent_id = %agent_id,
                    error = %e,
                    event = "lexical_prune_failed",
                );
            }
        }

        stats.duration_ms = start.elapsed().as_millis() as u64;

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            docs_scanned = stats.docs_scanned,
            docs_skipped = stats.docs_skipped_unchanged,
            docs_ingested = stats.docs_ingested,
            docs_pruned = stats.docs_pruned,
            chunks_added = stats.chunks_added,
            chunks_deleted = stats.chunks_deleted,
            duration_ms = stats.duration_ms,
            event = "lexical_ingest_complete",
        );

        Ok(stats)
    }

    /// Ingest a single document by absolute path.
    ///
    /// Called by the compound hook for immediate ingestion of freshly-written
    /// docs. Uses the same hash-check + ingest-if-changed logic as the bulk
    /// path.
    pub async fn ingest_single_doc(&self, path: &Path) -> Result<DocStats> {
        self.ingest_single_doc_inner(path).await
    }

    // -- Internal helpers --

    /// Core single-doc ingestion logic shared by `ingest_all` and
    /// `ingest_single_doc`.
    async fn ingest_single_doc_inner(&self, abs_path: &Path) -> Result<DocStats> {
        let raw = std::fs::read(abs_path)?;
        let normalized = normalize_content(&raw);

        if normalized.trim().is_empty() {
            return Ok(DocStats {
                outcome: DocOutcome::Skipped,
                chunks_added: 0,
                chunks_deleted: 0,
            });
        }

        let new_hash = compute_hash(&normalized);
        let rel_path = self
            .relative_path(abs_path)
            .unwrap_or_else(|| abs_path.to_string_lossy().to_string());

        // Check existing hash in DB.
        let agent_id = self.db.agent_id.clone();
        let rel_path_for_query = rel_path.clone();
        let existing_hash: Option<String> = self
            .db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT DISTINCT source_doc_hash FROM kg_chunks
                     WHERE agent_id = ?1 AND source_doc_path = ?2",
                )?;
                let hashes: Vec<String> = stmt
                    .query_map(rusqlite::params![agent_id, rel_path_for_query], |row| {
                        row.get(0)
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                if hashes.len() == 1 {
                    Ok(Some(hashes.into_iter().next().unwrap()))
                } else {
                    // Empty or multiple distinct hashes — force re-ingest.
                    Ok(None)
                }
            })
            .await?;

        // Skip if unchanged.
        if existing_hash.as_deref() == Some(&new_hash) {
            return Ok(DocStats {
                outcome: DocOutcome::Skipped,
                chunks_added: 0,
                chunks_deleted: 0,
            });
        }

        // Delete existing chunks for this doc (if any) before re-inserting.
        let chunks_deleted = if existing_hash.is_some() {
            self.delete_doc_chunks(&rel_path).await?
        } else {
            0
        };

        // Chunk and insert.
        let chunks = chunk_doc(&normalized);
        let chunks_added = chunks.len();

        let agent_id = self.db.agent_id.clone();
        let rel_path_for_insert = rel_path.clone();
        let new_hash_for_insert = new_hash;
        let trace_id = self.trace_id.clone();

        self.db
            .with_db(move |db| {
                let tx = db.conn.unchecked_transaction()?;

                for chunk in &chunks {
                    tx.execute(
                        "INSERT INTO kg_chunks (agent_id, seq_id, source_doc_path, source_doc_hash, trace_id)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            agent_id,
                            chunk.seq_id,
                            rel_path_for_insert,
                            new_hash_for_insert,
                            trace_id,
                        ],
                    )?;

                    let chunk_rowid = tx.last_insert_rowid();

                    // Index the chunk text for FTS + embedding backfill.
                    // Call index_content directly on the Database, which uses
                    // the same underlying connection (inside the transaction).
                    db.index_content(
                        &agent_id,
                        KG_CHUNK_SOURCE_TYPE,
                        Some(chunk_rowid),
                        &chunk.text,
                    )?;
                }

                tx.commit()?;
                Ok(())
            })
            .await?;

        Ok(DocStats {
            outcome: DocOutcome::Ingested,
            chunks_added,
            chunks_deleted,
        })
    }

    /// Delete all chunks for a document, including search_content cleanup.
    ///
    /// Returns the number of chunks deleted.
    async fn delete_doc_chunks(&self, rel_path: &str) -> Result<usize> {
        let agent_id = self.db.agent_id.clone();
        let rel_path = rel_path.to_owned();

        self.db
            .with_db(move |db| {
                let tx = db.conn.unchecked_transaction()?;

                // Get chunk IDs for this doc.
                let chunk_ids: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM kg_chunks WHERE agent_id = ?1 AND source_doc_path = ?2",
                    )?;
                    stmt.query_map(rusqlite::params![agent_id, rel_path], |row| row.get(0))?
                        .filter_map(|r| r.ok())
                        .collect()
                };

                // For each chunk, clean up search_content + FTS + vec.
                for &chunk_id in &chunk_ids {
                    let existing: Option<(i64, String)> = tx
                        .query_row(
                            "SELECT id, content FROM search_content
                              WHERE agent_id = ?1 AND source_type = ?2 AND source_id = ?3",
                            rusqlite::params![agent_id, KG_CHUNK_SOURCE_TYPE, chunk_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;

                    if let Some((sc_id, content)) = existing {
                        let _ = tx.execute(
                            "INSERT INTO fts_search(fts_search, rowid, content) VALUES ('delete', ?1, ?2)",
                            rusqlite::params![sc_id, content],
                        );
                        let _ = tx.execute(
                            "DELETE FROM vec_search WHERE rowid = ?1",
                            rusqlite::params![sc_id],
                        );
                        tx.execute(
                            "DELETE FROM search_content WHERE id = ?1",
                            rusqlite::params![sc_id],
                        )?;
                    }
                }

                let count = chunk_ids.len();
                tx.execute(
                    "DELETE FROM kg_chunks WHERE agent_id = ?1 AND source_doc_path = ?2",
                    rusqlite::params![agent_id, rel_path],
                )?;

                tx.commit()?;
                Ok(count)
            })
            .await
    }

    /// Find and delete chunks for docs no longer on disk.
    async fn prune_removed_docs(&self, on_disk_paths: &HashSet<String>) -> Result<(usize, usize)> {
        // Get all tracked doc paths from DB.
        let agent_id = self.db.agent_id.clone();
        let tracked_paths: Vec<String> = self
            .db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT DISTINCT source_doc_path FROM kg_chunks WHERE agent_id = ?1",
                )?;
                let paths: Vec<String> = stmt
                    .query_map(rusqlite::params![agent_id], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(paths)
            })
            .await?;

        let mut pruned = 0;
        let mut deleted_chunks = 0;
        for path in tracked_paths {
            if !on_disk_paths.contains(&path) {
                match self.delete_doc_chunks(&path).await {
                    Ok(count) => {
                        pruned += 1;
                        deleted_chunks += count;

                        // Emit prune audit event (C3.2).
                        let prune_stats = DocStats {
                            outcome: DocOutcome::Pruned,
                            chunks_added: 0,
                            chunks_deleted: count,
                        };
                        self.emit_audit_event(&path, &prune_stats).await;
                    }
                    Err(e) => {
                        warn!(
                            trace_id = %self.trace_id,
                            doc = %path,
                            error = %e,
                            event = "lexical_prune_doc_failed",
                        );
                    }
                }
            }
        }

        Ok((pruned, deleted_chunks))
    }

    /// Emit a per-document audit event per C3.2.
    ///
    /// Fire-and-forget: failures are logged but do not propagate.
    async fn emit_audit_event(&self, rel_path: &str, doc_stats: &DocStats) {
        let outcome = match doc_stats.outcome {
            DocOutcome::Skipped => "skipped",
            DocOutcome::Ingested => "ingested",
            DocOutcome::Pruned => "pruned",
        };

        let after_value = format!(
            r#"{{"chunks_added":{},"chunks_deleted":{}}}"#,
            doc_stats.chunks_added, doc_stats.chunks_deleted,
        );

        let target_key = format!("kg_chunk:{rel_path}");

        if let Err(e) = self
            .db
            .log_audit_event(
                &self.session_id,
                "ingest_document",
                &target_key,
                None,
                Some(&after_value),
                Some(outcome),
                Some(&self.trace_id),
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                doc = %rel_path,
                error = %e,
                event = "lexical_audit_event_failed",
            );
        }
    }

    /// Walk `docs_root` recursively for `*.md` files.
    fn scan_docs(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        if self.docs_root.exists() {
            self.walk_dir(&self.docs_root, &mut files)?;
        }
        files.sort(); // Deterministic ordering.
        Ok(files)
    }

    fn walk_dir(&self, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.walk_dir(&path, out)?;
            } else if path.extension().is_some_and(|ext| ext == "md") {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Convert an absolute path to a repo-relative path.
    ///
    /// `docs_root` is typically `<repo>/docs/solutions`, so the repo root is
    /// two levels up. Paths stored in DB are always repo-relative (e.g.,
    /// `docs/solutions/database-issues/foo.md`).
    fn relative_path(&self, abs_path: &Path) -> Option<String> {
        if let Some(repo_root) = self.docs_root.parent().and_then(|p| p.parent()) {
            abs_path
                .strip_prefix(repo_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            abs_path
                .strip_prefix(&self.docs_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        }
    }
}

/// Normalize document content for consistent hashing.
///
/// 1. Strip UTF-8 BOM if present.
/// 2. Normalize line endings to LF (replace `\r\n` and `\r` with `\n`).
/// 3. Strip trailing whitespace from each line.
/// 4. Enforce single trailing newline.
pub fn normalize_content(raw: &[u8]) -> String {
    // Strip BOM.
    let bytes = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &raw[3..]
    } else {
        raw
    };

    // Decode as UTF-8 (lossy — invalid sequences become U+FFFD).
    let text = String::from_utf8_lossy(bytes);

    // Normalize line endings: \r\n -> \n, then lone \r -> \n.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");

    // Strip trailing whitespace from each line.
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    let mut result = lines.join("\n");

    // Enforce single trailing newline.
    let trimmed = result.trim_end_matches('\n');
    result = format!("{trimmed}\n");

    result
}

/// Compute SHA-256 hex digest of normalized content.
pub fn compute_hash(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_bom() {
        let with_bom = b"\xEF\xBB\xBFhello\n";
        assert_eq!(normalize_content(with_bom), "hello\n");
    }

    #[test]
    fn normalize_crlf_to_lf() {
        let crlf = b"line1\r\nline2\r\n";
        assert_eq!(normalize_content(crlf), "line1\nline2\n");
    }

    #[test]
    fn normalize_lone_cr_to_lf() {
        let cr = b"line1\rline2\r";
        assert_eq!(normalize_content(cr), "line1\nline2\n");
    }

    #[test]
    fn normalize_strips_trailing_ws() {
        let text = b"hello   \nworld  \n";
        assert_eq!(normalize_content(text), "hello\nworld\n");
    }

    #[test]
    fn normalize_enforces_single_trailing_newline() {
        assert_eq!(normalize_content(b"hello\n\n\n"), "hello\n");
        assert_eq!(normalize_content(b"hello"), "hello\n");
    }

    #[test]
    fn compute_hash_deterministic() {
        let h1 = compute_hash("hello\n");
        let h2 = compute_hash("hello\n");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn hash_changes_on_different_content() {
        let h1 = compute_hash("hello\n");
        let h2 = compute_hash("world\n");
        assert_ne!(h1, h2);
    }
}
