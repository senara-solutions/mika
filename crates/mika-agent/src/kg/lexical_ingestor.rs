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
    pub docs_index_backfilled: usize,
    pub chunks_added: usize,
    pub chunks_deleted: usize,
    pub chunks_indexed_backfill: usize,
    pub duration_ms: u64,
}

/// Statistics returned from a single-doc ingestion.
#[derive(Debug, Clone)]
pub struct DocStats {
    pub outcome: DocOutcome,
    pub chunks_added: usize,
    pub chunks_deleted: usize,
    pub chunks_indexed_backfill: usize,
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
    /// Doc was unchanged but per-agent search index was missing — backfilled.
    IndexBackfilled,
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
    /// Precomputed `hash_docs_root(&docs_root)` — used as the shared-corpus
    /// key in v27 KG tables instead of `agent_id`.
    docs_root_hash: String,
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
        let docs_root_hash = super::config::hash_docs_root(&docs_root);
        Self {
            db,
            docs_root,
            docs_root_hash,
            trace_id,
            session_id: STARTUP_SESSION_ID.to_owned(),
        }
    }

    /// Returns the precomputed docs_root_hash (shared-corpus key for v27 KG tables).
    pub fn docs_root_hash(&self) -> &str {
        &self.docs_root_hash
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
            docs_index_backfilled: 0,
            chunks_added: 0,
            chunks_deleted: 0,
            chunks_indexed_backfill: 0,
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
                        DocOutcome::IndexBackfilled => {
                            stats.docs_index_backfilled += 1;
                            stats.chunks_indexed_backfill += doc_stats.chunks_indexed_backfill;
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
            docs_index_backfilled = stats.docs_index_backfilled,
            chunks_added = stats.chunks_added,
            chunks_deleted = stats.chunks_deleted,
            chunks_indexed_backfill = stats.chunks_indexed_backfill,
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
    ///
    /// All DB mutations (hash check → delete old chunks → insert new chunks →
    /// index for FTS) happen in a **single `with_db` transaction** so readers
    /// never see a flash-of-empty state for the document.
    async fn ingest_single_doc_inner(&self, abs_path: &Path) -> Result<DocStats> {
        let raw = std::fs::read(abs_path)?;
        let normalized = normalize_content(&raw);
        let rel_path = self
            .relative_path(abs_path)
            .unwrap_or_else(|| abs_path.to_string_lossy().to_string());

        // If the file is empty after normalization, clean up any stale chunks
        // from a previous run where the doc had content.
        if normalized.trim().is_empty() {
            let chunks_deleted = self.delete_doc_chunks(&rel_path).await.unwrap_or(0);
            return Ok(DocStats {
                outcome: DocOutcome::Skipped,
                chunks_added: 0,
                chunks_deleted,
                chunks_indexed_backfill: 0,
            });
        }

        let new_hash = compute_hash(&normalized);
        let chunks = chunk_doc(&normalized);

        let agent_id = self.db.agent_id.clone();
        let docs_root_hash = self.docs_root_hash.clone();
        let docs_root_display = self.docs_root.display().to_string();
        let rel_path_owned = rel_path.clone();
        let new_hash_owned = new_hash;
        let trace_id = self.trace_id.clone();

        // Single with_db call: hash check + delete + insert are one transaction.
        // This eliminates the empty-window gap between delete and insert.
        //
        // Returns: (chunks_added, chunks_deleted, outcome_kind, chunks_backfilled)
        // where outcome_kind: 0=ingested, 1=skipped, 2=index_backfilled
        let (chunks_added, chunks_deleted, outcome_kind, chunks_backfilled) = self
            .db
            .with_db(move |db| {
                // 1. Hash check: is the doc unchanged?
                let existing_hashes: Vec<String> = {
                    let mut stmt = db.conn.prepare(
                        "SELECT DISTINCT source_doc_hash FROM kg_chunks
                         WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
                    )?;
                    stmt.query_map(
                        rusqlite::params![docs_root_hash, rel_path_owned],
                        |row| row.get(0),
                    )?
                    .filter_map(|r| r.ok())
                    .collect()
                };

                // Single matching hash → unchanged. Check per-agent search index.
                if existing_hashes.len() == 1 && existing_hashes[0] == new_hash_owned {
                    // Two-axis idempotency: chunks are shared (docs_root_hash scope),
                    // but search_content is per-agent. Check if this agent has
                    // search_content rows for all existing chunks.
                    //
                    // kg_chunks has no `text` column — chunk text is only in
                    // search_content.content. We use the in-memory chunker output
                    // (deterministic for same content hash) paired with DB chunk IDs.
                    let db_chunk_ids: Vec<i64> = {
                        let mut stmt = db.conn.prepare(
                            "SELECT id FROM kg_chunks
                             WHERE docs_root_hash = ?1 AND source_doc_path = ?2
                             ORDER BY seq_id",
                        )?;
                        stmt.query_map(
                            rusqlite::params![docs_root_hash, rel_path_owned],
                            |row| row.get(0),
                        )?
                        .filter_map(|r| r.ok())
                        .collect()
                    };

                    // D3a: fail-loud on row-count mismatch between DB and in-memory chunker.
                    // If they disagree, something is wrong (chunker version drift) —
                    // fall through to the delete+insert path.
                    if db_chunk_ids.len() != chunks.len() {
                        warn!(
                            trace_id = %trace_id,
                            agent_id = %agent_id,
                            doc = %rel_path_owned,
                            db_count = db_chunk_ids.len(),
                            memory_count = chunks.len(),
                            event = "lexical_backfill_chunk_count_mismatch",
                        );
                        // Fall through to delete+insert path below.
                    } else {
                        // Find which chunks are missing from this agent's search_content.
                        let missing_indices: Vec<usize> = if db_chunk_ids.is_empty() {
                            Vec::new()
                        } else {
                            // Query agent's existing search_content source_ids for these chunks.
                            let placeholders: String = db_chunk_ids
                                .iter()
                                .map(|_| "?")
                                .collect::<Vec<_>>()
                                .join(",");
                            let sql = format!(
                                "SELECT source_id FROM search_content
                                 WHERE agent_id = ?1 AND source_type = ?2
                                   AND source_id IN ({})",
                                placeholders
                            );
                            let mut stmt = db.conn.prepare(&sql)?;
                            // Bind agent_id and source_type, then chunk IDs.
                            let mut param_idx = 1;
                            stmt.raw_bind_parameter(param_idx, &agent_id)?;
                            param_idx += 1;
                            stmt.raw_bind_parameter(param_idx, KG_CHUNK_SOURCE_TYPE)?;
                            param_idx += 1;
                            for &cid in &db_chunk_ids {
                                stmt.raw_bind_parameter(param_idx, cid)?;
                                param_idx += 1;
                            }
                            let existing_ids: HashSet<i64> = {
                                let mut rows = stmt.raw_query();
                                let mut set = HashSet::new();
                                while let Some(row) = rows.next()? {
                                    let id: i64 = row.get(0)?;
                                    set.insert(id);
                                }
                                set
                            };

                            db_chunk_ids
                                .iter()
                                .enumerate()
                                .filter(|(_, id)| !existing_ids.contains(id))
                                .map(|(idx, _)| idx)
                                .collect()
                        };

                        if missing_indices.is_empty() {
                            // True no-op: chunks exist and agent has all search rows.
                            return Ok((0usize, 0usize, 1u8, 0usize));
                        }

                        // Backfill: write missing search_content rows in a transaction.
                        // Text comes from the in-memory chunker (deterministic for same hash).
                        let backfill_count = missing_indices.len();
                        let tx = db.conn.unchecked_transaction()?;
                        for &idx in &missing_indices {
                            let chunk_id = db_chunk_ids[idx];
                            let text = &chunks[idx].text;
                            db.index_content(
                                &agent_id,
                                KG_CHUNK_SOURCE_TYPE,
                                Some(chunk_id),
                                text,
                            )?;
                        }
                        tx.commit()?;

                        return Ok((0usize, 0usize, 2u8, backfill_count));
                    }
                }

                // 2. Atomic delete + insert in one transaction.
                let tx = db.conn.unchecked_transaction()?;

                // Delete existing chunks (if any). Always delete when chunks
                // exist — handles both the normal re-ingest case and the
                // inconsistent-hashes corruption case.
                let mut deleted = 0usize;
                if !existing_hashes.is_empty() {
                    // Clean up search_content/FTS/vec for each chunk.
                    let chunk_ids: Vec<i64> = {
                        let mut stmt = tx.prepare(
                            "SELECT id FROM kg_chunks WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
                        )?;
                        stmt.query_map(
                            rusqlite::params![docs_root_hash, rel_path_owned],
                            |row| row.get(0),
                        )?
                        .filter_map(|r| r.ok())
                        .collect()
                    };

                    for &chunk_id in &chunk_ids {
                        // Call delete_search_content on the Database which uses
                        // the same connection as the transaction (unchecked_transaction
                        // borrows the connection, so db.conn writes are inside tx).
                        db.delete_search_content(&agent_id, KG_CHUNK_SOURCE_TYPE, chunk_id)?;
                    }

                    deleted = chunk_ids.len();
                    tx.execute(
                        "DELETE FROM kg_chunks WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
                        rusqlite::params![docs_root_hash, rel_path_owned],
                    )?;
                }

                // 3. Insert new chunks + index_content in the same transaction.
                for chunk in &chunks {
                    tx.execute(
                        "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash, trace_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            docs_root_hash,
                            docs_root_display,
                            chunk.seq_id,
                            rel_path_owned,
                            new_hash_owned,
                            trace_id,
                        ],
                    )?;

                    let chunk_rowid = tx.last_insert_rowid();

                    // SAFETY: db.index_content uses db.conn which is the same
                    // connection that tx wraps (via unchecked_transaction). All
                    // writes through db.conn are inside this transaction and
                    // will commit or rollback atomically.
                    db.index_content(
                        &agent_id,
                        KG_CHUNK_SOURCE_TYPE,
                        Some(chunk_rowid),
                        &chunk.text,
                    )?;
                }

                tx.commit()?;
                Ok((chunks.len(), deleted, 0u8, 0usize))
            })
            .await?;

        match outcome_kind {
            1 => Ok(DocStats {
                outcome: DocOutcome::Skipped,
                chunks_added: 0,
                chunks_deleted: 0,
                chunks_indexed_backfill: 0,
            }),
            2 => Ok(DocStats {
                outcome: DocOutcome::IndexBackfilled,
                chunks_added: 0,
                chunks_deleted: 0,
                chunks_indexed_backfill: chunks_backfilled,
            }),
            _ => Ok(DocStats {
                outcome: DocOutcome::Ingested,
                chunks_added,
                chunks_deleted,
                chunks_indexed_backfill: 0,
            }),
        }
    }

    /// Delete all chunks for a document, including search_content cleanup.
    ///
    /// Uses [`Database::delete_search_content`] for the symmetric cleanup of
    /// `search_content` + `fts_search` + `vec_search` to avoid duplicating
    /// delete logic that would diverge when new search tables are added.
    ///
    /// Returns the number of chunks deleted.
    async fn delete_doc_chunks(&self, rel_path: &str) -> Result<usize> {
        let agent_id = self.db.agent_id.clone();
        let docs_root_hash = self.docs_root_hash.clone();
        let rel_path = rel_path.to_owned();

        self.db
            .with_db(move |db| {
                let tx = db.conn.unchecked_transaction()?;

                // Get chunk IDs for this doc.
                let chunk_ids: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM kg_chunks WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
                    )?;
                    stmt.query_map(rusqlite::params![docs_root_hash, rel_path], |row| {
                        row.get(0)
                    })?
                    .filter_map(|r| r.ok())
                    .collect()
                };

                // Symmetric delete: use the canonical delete_search_content which
                // handles search_content + fts_search + vec_search cleanup.
                // db.delete_search_content uses db.conn which is the same connection
                // as tx (via unchecked_transaction), so writes are transactional.
                for &chunk_id in &chunk_ids {
                    db.delete_search_content(&agent_id, KG_CHUNK_SOURCE_TYPE, chunk_id)?;
                }

                let count = chunk_ids.len();
                tx.execute(
                    "DELETE FROM kg_chunks WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
                    rusqlite::params![docs_root_hash, rel_path],
                )?;

                tx.commit()?;
                Ok(count)
            })
            .await
    }

    /// Find and delete chunks for docs no longer on disk.
    async fn prune_removed_docs(&self, on_disk_paths: &HashSet<String>) -> Result<(usize, usize)> {
        // Get all tracked doc paths from DB.
        let docs_root_hash = self.docs_root_hash.clone();
        let tracked_paths: Vec<String> = self
            .db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT DISTINCT source_doc_path FROM kg_chunks WHERE docs_root_hash = ?1",
                )?;
                let paths: Vec<String> = stmt
                    .query_map(rusqlite::params![docs_root_hash], |row| row.get(0))?
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
                            chunks_indexed_backfill: 0,
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
            DocOutcome::IndexBackfilled => "index_backfilled",
        };

        let after_value = format!(
            r#"{{"chunks_added":{},"chunks_deleted":{},"chunks_indexed_backfill":{}}}"#,
            doc_stats.chunks_added, doc_stats.chunks_deleted, doc_stats.chunks_indexed_backfill,
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
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;
    use crate::db::kg_schema::KG_CHUNK_SOURCE_TYPE;

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

    // -- Backfill tests (#1155) --

    /// Helper: create a temp dir with a docs/solutions structure and a single .md file.
    /// Returns (temp_dir_guard, docs_root_path).
    fn setup_test_docs(content: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let docs_root = tmp.path().join("docs").join("solutions");
        std::fs::create_dir_all(&docs_root).unwrap();
        std::fs::write(docs_root.join("test-doc.md"), content).unwrap();
        (tmp, docs_root)
    }

    /// Helper: count search_content rows for an agent + source_type using with_db.
    async fn count_search_content_async(db: &AsyncDatabase, agent_id: &str) -> usize {
        let agent_id = agent_id.to_owned();
        db.with_db(move |db| {
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM search_content WHERE agent_id = ?1 AND source_type = ?2",
                    rusqlite::params![agent_id, KG_CHUNK_SOURCE_TYPE],
                    |row| row.get::<_, usize>(0),
                )
                .map_err(Into::into)
        })
        .await
        .unwrap()
    }

    /// Helper: ensure an agent exists in the agents table.
    async fn ensure_agent_async(db: &AsyncDatabase, agent_id: &str) {
        let agent_id = agent_id.to_owned();
        db.with_db(move |db| {
            db.conn
                .execute(
                    "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES (?1, ?1, '')",
                    rusqlite::params![agent_id],
                )
                .map_err(Into::into)
        })
        .await
        .unwrap();
    }

    /// Test 1: agent B ingests a doc after agent A already ingested it.
    /// Agent B should get search_content rows via backfill.
    #[tokio::test]
    async fn backfill_when_second_agent_finds_chunks_already_indexed() {
        let doc_content = "---\ntitle: Test\n---\n\n## Section One\n\nSome content here.\n\n## Section Two\n\nMore content here.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let db_a = AsyncDatabase::new_with_agent(db, "agent-a");
        ensure_agent_async(&db_a, "agent-a").await;
        ensure_agent_async(&db_a, "agent-b").await;

        // Agent A ingests first.
        let ingestor_a = LexicalIngestor::new(db_a.clone(), docs_root.clone(), None);
        let stats_a = ingestor_a.ingest_all().await.unwrap();
        assert!(stats_a.docs_ingested > 0);
        assert!(stats_a.chunks_added > 0);
        let a_count = count_search_content_async(&db_a, "agent-a").await;
        assert!(a_count > 0, "agent-a should have search_content rows");

        // Agent B ingests — same docs_root → shared corpus, same DB connection.
        let db_b = db_a.with_agent("agent-b");
        let ingestor_b = LexicalIngestor::new(db_b.clone(), docs_root.clone(), None);
        let stats_b = ingestor_b.ingest_all().await.unwrap();

        // Agent B should see backfill, not fresh ingest.
        assert_eq!(stats_b.docs_index_backfilled, 1);
        assert_eq!(stats_b.docs_ingested, 0);
        assert_eq!(stats_b.docs_skipped_unchanged, 0);
        assert!(stats_b.chunks_indexed_backfill > 0);

        let b_count = count_search_content_async(&db_b, "agent-b").await;
        assert_eq!(
            b_count, a_count,
            "agent-b should have same count as agent-a"
        );
    }

    /// Test 2: agent A re-ingests — should be a true no-op (Skipped).
    #[tokio::test]
    async fn no_backfill_when_agent_already_has_search_rows() {
        let doc_content = "---\ntitle: Test\n---\n\n## Section\n\nContent.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "mika");

        let ingestor = LexicalIngestor::new(async_db.clone(), docs_root.clone(), None);

        // First ingest.
        let stats1 = ingestor.ingest_all().await.unwrap();
        assert!(stats1.docs_ingested > 0);

        // Second ingest — true skip.
        let stats2 = ingestor.ingest_all().await.unwrap();
        assert_eq!(stats2.docs_skipped_unchanged, 1);
        assert_eq!(stats2.docs_index_backfilled, 0);
        assert_eq!(stats2.chunks_indexed_backfill, 0);
    }

    /// Test 3: agent B has a subset of chunks pre-seeded — only missing ones are backfilled.
    #[tokio::test]
    async fn partial_backfill_when_agent_has_subset_of_chunks() {
        let doc_content = "---\ntitle: Test\n---\n\n## Section One\n\nContent one.\n\n## Section Two\n\nContent two.\n\n## Section Three\n\nContent three.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let db_a = AsyncDatabase::new_with_agent(db, "agent-a");
        ensure_agent_async(&db_a, "agent-a").await;
        ensure_agent_async(&db_a, "agent-b").await;

        // Agent A ingests first.
        let ingestor_a = LexicalIngestor::new(db_a.clone(), docs_root.clone(), None);
        let stats_a = ingestor_a.ingest_all().await.unwrap();
        let total_chunks = stats_a.chunks_added;
        assert!(total_chunks >= 3, "should have at least 3 chunks");

        // Pre-seed ONE chunk for agent-b.
        let docs_root_hash = super::super::config::hash_docs_root(&docs_root);
        let docs_root_hash_clone = docs_root_hash.clone();
        db_a.with_db(move |db| {
            let first_chunk_id: i64 = db
                .conn
                .query_row(
                    "SELECT id FROM kg_chunks WHERE docs_root_hash = ?1 ORDER BY seq_id LIMIT 1",
                    rusqlite::params![docs_root_hash_clone],
                    |row| row.get(0),
                )?;

            let first_chunk_text: String = db
                .conn
                .query_row(
                    "SELECT content FROM search_content WHERE agent_id = 'agent-a' AND source_type = ?1 AND source_id = ?2",
                    rusqlite::params![KG_CHUNK_SOURCE_TYPE, first_chunk_id],
                    |row| row.get(0),
                )?;

            db.index_content("agent-b", KG_CHUNK_SOURCE_TYPE, Some(first_chunk_id), &first_chunk_text)?;
            Ok(())
        })
        .await
        .unwrap();

        let pre_count = count_search_content_async(&db_a, "agent-b").await;
        assert_eq!(pre_count, 1);

        // Agent B ingests — should backfill only the missing chunks.
        let db_b = db_a.with_agent("agent-b");
        let ingestor_b = LexicalIngestor::new(db_b.clone(), docs_root.clone(), None);
        let stats_b = ingestor_b.ingest_all().await.unwrap();

        assert_eq!(stats_b.docs_index_backfilled, 1);
        assert_eq!(
            stats_b.chunks_indexed_backfill,
            total_chunks - 1,
            "should backfill all but the pre-seeded chunk"
        );

        let post_count = count_search_content_async(&db_b, "agent-b").await;
        assert_eq!(
            post_count, total_chunks,
            "agent-b should have all chunks now"
        );
    }

    /// Test 4: backfill is idempotent — second run after backfill is a true skip.
    #[tokio::test]
    async fn backfill_preserves_idempotency_on_repeat() {
        let doc_content = "---\ntitle: Test\n---\n\n## Section\n\nContent.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let db_a = AsyncDatabase::new_with_agent(db, "agent-a");
        ensure_agent_async(&db_a, "agent-a").await;
        ensure_agent_async(&db_a, "agent-b").await;

        // Agent A ingests.
        let ingestor_a = LexicalIngestor::new(db_a.clone(), docs_root.clone(), None);
        ingestor_a.ingest_all().await.unwrap();

        // Agent B first run — backfill.
        let db_b = db_a.with_agent("agent-b");
        let ingestor_b = LexicalIngestor::new(db_b.clone(), docs_root.clone(), None);
        let stats1 = ingestor_b.ingest_all().await.unwrap();
        assert_eq!(stats1.docs_index_backfilled, 1);

        // Agent B second run — true skip (not backfill again).
        let stats2 = ingestor_b.ingest_all().await.unwrap();
        assert_eq!(stats2.docs_skipped_unchanged, 1);
        assert_eq!(stats2.docs_index_backfilled, 0);
        assert_eq!(stats2.chunks_indexed_backfill, 0);
    }

    /// Test 5: FTS search works after backfill.
    #[tokio::test]
    async fn fts_search_rows_present_after_backfill() {
        let doc_content =
            "---\ntitle: Test\n---\n\n## Section\n\nThe magic needle keyword appears here.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let db_a = AsyncDatabase::new_with_agent(db, "agent-a");
        ensure_agent_async(&db_a, "agent-a").await;
        ensure_agent_async(&db_a, "agent-b").await;

        // Agent A ingests.
        let ingestor_a = LexicalIngestor::new(db_a.clone(), docs_root.clone(), None);
        ingestor_a.ingest_all().await.unwrap();

        // Agent B backfills.
        let db_b = db_a.with_agent("agent-b");
        let ingestor_b = LexicalIngestor::new(db_b.clone(), docs_root.clone(), None);
        let stats = ingestor_b.ingest_all().await.unwrap();
        assert!(stats.docs_index_backfilled > 0);

        // FTS search for agent-b should find the keyword.
        let results = db_b
            .with_db(move |db| db.fts_search("agent-b", "needle", 10, Some(KG_CHUNK_SOURCE_TYPE)))
            .await
            .unwrap();
        assert!(
            !results.is_empty(),
            "FTS search should find 'needle' for agent-b after backfill"
        );
    }

    /// Test 6: get_chunk_context join shape succeeds after backfill.
    #[tokio::test]
    async fn get_chunk_context_join_succeeds_post_backfill() {
        let doc_content = "---\ntitle: Test\n---\n\n## Section\n\nContent for join test.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let db_a = AsyncDatabase::new_with_agent(db, "agent-a");
        ensure_agent_async(&db_a, "agent-a").await;
        ensure_agent_async(&db_a, "agent-b").await;

        // Agent A ingests.
        let ingestor_a = LexicalIngestor::new(db_a.clone(), docs_root.clone(), None);
        ingestor_a.ingest_all().await.unwrap();

        let docs_root_hash = super::super::config::hash_docs_root(&docs_root);
        let drh = docs_root_hash.clone();

        // Seed a kg_chunk_subjects row.
        let (_chunk_id, subject_entity_id) = db_a
            .with_db(move |db| {
                let (chunk_id, docs_root): (i64, String) = db.conn.query_row(
                    "SELECT id, docs_root FROM kg_chunks WHERE docs_root_hash = ?1 LIMIT 1",
                    rusqlite::params![drh],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;

                db.conn.execute(
                    "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence)
                     VALUES (?1, ?2, 'concept:test_entity', 'concept', 'test_entity', 0.9)",
                    rusqlite::params![drh, docs_root],
                )?;
                let subject_entity_id = db.conn.last_insert_rowid();

                db.conn.execute(
                    "INSERT INTO kg_chunk_subjects (docs_root_hash, docs_root, chunk_id, subject_entity_id)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![drh, docs_root, chunk_id, subject_entity_id],
                )?;

                Ok((chunk_id, subject_entity_id))
            })
            .await
            .unwrap();

        // Agent B backfills.
        let db_b = db_a.with_agent("agent-b");
        let ingestor_b = LexicalIngestor::new(db_b.clone(), docs_root.clone(), None);
        ingestor_b.ingest_all().await.unwrap();

        // Run the get_chunk_context join shape for agent-b.
        let drh2 = docs_root_hash.clone();
        let result: Option<String> = db_b
            .with_db(move |db| {
                use rusqlite::OptionalExtension;
                db.conn
                    .query_row(
                        "SELECT sc.content
                         FROM kg_chunk_subjects cs
                         JOIN search_content sc ON sc.source_type = 'kg_chunk'
                             AND sc.source_id = cs.chunk_id AND sc.agent_id = ?1
                         WHERE cs.docs_root_hash = ?2 AND cs.subject_entity_id = ?3
                         LIMIT 1",
                        rusqlite::params!["agent-b", drh2, subject_entity_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        assert!(
            result.is_some(),
            "get_chunk_context join should find content for agent-b after backfill"
        );
    }

    /// Test 7: changed content takes the normal ingest path, not backfill.
    #[tokio::test]
    async fn changed_content_takes_normal_ingest_path_not_backfill() {
        let doc_content_v1 = "---\ntitle: V1\n---\n\n## Section\n\nOriginal.\n";
        let (_tmp, docs_root) = setup_test_docs(doc_content_v1);

        let tmp_db = tempfile::NamedTempFile::new().unwrap();
        let db = Database::open(tmp_db.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "mika");

        let ingestor = LexicalIngestor::new(async_db.clone(), docs_root.clone(), None);

        // First ingest.
        let stats1 = ingestor.ingest_all().await.unwrap();
        assert_eq!(stats1.docs_ingested, 1);

        // Change the doc content.
        std::fs::write(
            docs_root.join("test-doc.md"),
            "---\ntitle: V2\n---\n\n## Section\n\nUpdated content here.\n",
        )
        .unwrap();

        // Re-ingest — should be Ingested, not IndexBackfilled.
        let stats2 = ingestor.ingest_all().await.unwrap();
        assert_eq!(stats2.docs_ingested, 1);
        assert_eq!(stats2.docs_index_backfilled, 0);
        assert_eq!(stats2.chunks_indexed_backfill, 0);
    }
}
