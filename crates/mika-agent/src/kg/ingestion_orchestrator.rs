//! # Ingestion Orchestrator
//!
//! Coordinates the lexical ingestor (#689) and subject extractor (#690).
//! Neither module has knowledge of the other's internals — the orchestrator
//! owns the sequencing (per D9 in the #690 plan).
//!
//! ## Responsibilities
//!
//! 1. **Compound hook** — after a doc is written, ingest chunks then extract
//!    subjects synchronously inline.
//! 2. **Re-extraction on doc change** — three-phase capture → reingest →
//!    reconcile (D5).
//!
//! ## Sole-Writer Separation
//!
//! - `LexicalIngestor` is the sole writer of `kg_chunks` and `search_content`.
//! - `SubjectExtractor` is the sole writer of `kg_subject_entities`,
//!   `kg_subject_relationships`, and provenance tables.
//! - This module calls both but writes nothing itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use mika_common::llm::LlmProvider;

use crate::async_db::AsyncDatabase;

use super::lexical_ingestor::LexicalIngestor;
use super::subject_extractor::SubjectExtractor;

/// Coordinates lexical ingestion and subject extraction.
///
/// Provides the compound hook entry point and re-extraction flow.
pub struct IngestionOrchestrator {
    db: AsyncDatabase,
    docs_root: PathBuf,
    /// LLM provider for extraction. `None` when extraction is disabled
    /// (no `MIKA_KG_EXTRACTION_MODEL` configured).
    extraction_llm: Option<Arc<dyn LlmProvider>>,
    trace_id: String,
    session_id: String,
}

impl IngestionOrchestrator {
    /// Create a new orchestrator.
    ///
    /// - `extraction_llm`: `None` to disable extraction (lexical ingestion
    ///   still runs).
    pub fn new(
        db: AsyncDatabase,
        docs_root: PathBuf,
        extraction_llm: Option<Arc<dyn LlmProvider>>,
        trace_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Self {
        let trace_id = trace_id
            .map(|s| s.to_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));
        Self {
            db,
            docs_root,
            extraction_llm,
            trace_id,
            session_id: session_id.unwrap_or("compound").to_owned(),
        }
    }

    /// Compound hook: ingest a freshly-written document, then extract subjects.
    ///
    /// Called after `/ce:compound` writes a doc. Lexical ingestion is synchronous
    /// and must succeed. Subject extraction is synchronous but failure is
    /// non-fatal (log-and-skip per C2.3) — the doc stays pending for next
    /// startup extraction.
    pub async fn ingest_and_extract(&self, abs_path: &Path) -> Result<CompoundResult> {
        let agent_id = self.db.agent_id.clone();

        // 1. Lexical ingestion (synchronous, must succeed).
        let ingestor = LexicalIngestor::new(
            self.db.clone(),
            self.docs_root.clone(),
            Some(&self.trace_id),
        )
        .with_session(&self.session_id);

        let ingest_stats = ingestor.ingest_single_doc(abs_path).await?;

        let rel_path = self.relative_path(abs_path);

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            doc = %rel_path,
            outcome = ?ingest_stats.outcome,
            chunks_added = ingest_stats.chunks_added,
            event = "compound_ingest_complete",
        );

        // 2. Subject extraction (synchronous, non-fatal).
        let extraction_stats = if let Some(ref llm) = self.extraction_llm {
            let extractor = SubjectExtractor::new(
                self.db.clone(),
                llm.clone(),
                self.docs_root.clone(),
                Some(&self.trace_id),
            );

            match extractor.extract_document(&rel_path, None).await {
                Ok(stats) => {
                    // Record extraction so it's not re-extracted at startup.
                    extractor
                        .record_extraction_public(
                            &rel_path,
                            llm.model_name(),
                            stats.entities_upserted,
                            stats.relationships_upserted,
                        )
                        .await;
                    Some(stats)
                }
                Err(e) => {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %rel_path,
                        error = %e,
                        event = "compound_extraction_failed",
                        "extraction failed — doc stays pending per C2.3"
                    );
                    None
                }
            }
        } else {
            None
        };

        Ok(CompoundResult {
            ingest_stats,
            extraction_stats,
        })
    }

    /// Re-extraction on doc change: three-phase capture → reingest → reconcile.
    ///
    /// Phase 1: Capture previous provenance state.
    /// Phase 2: Re-ingest chunks (handled by `LexicalIngestor`).
    /// Phase 3: Re-extract with previous state for orphan sweep.
    pub async fn reingest_and_reextract(&self, abs_path: &Path) -> Result<CompoundResult> {
        let rel_path = self.relative_path(abs_path);

        // Phase 1: Capture previous state before any deletion.
        let previous_state = if let Some(ref llm) = self.extraction_llm {
            let extractor = SubjectExtractor::new(
                self.db.clone(),
                llm.clone(),
                self.docs_root.clone(),
                Some(&self.trace_id),
            );
            Some(extractor.capture_previous_state(&rel_path).await?)
        } else {
            None
        };

        // Phase 2: Re-ingest (deletes old chunks, writes new chunks).
        let ingestor = LexicalIngestor::new(
            self.db.clone(),
            self.docs_root.clone(),
            Some(&self.trace_id),
        )
        .with_session(&self.session_id);

        let ingest_stats = ingestor.ingest_single_doc(abs_path).await?;

        // Phase 3: Re-extract with previous state for orphan sweep.
        let extraction_stats = match (&self.extraction_llm, previous_state) {
            (Some(llm), Some(prev)) => {
                let extractor = SubjectExtractor::new(
                    self.db.clone(),
                    llm.clone(),
                    self.docs_root.clone(),
                    Some(&self.trace_id),
                );

                match extractor.extract_document(&rel_path, Some(prev)).await {
                    Ok(stats) => {
                        extractor
                            .record_extraction_public(
                                &rel_path,
                                llm.model_name(),
                                stats.entities_upserted,
                                stats.relationships_upserted,
                            )
                            .await;
                        Some(stats)
                    }
                    Err(e) => {
                        warn!(
                            trace_id = %self.trace_id,
                            doc = %rel_path,
                            error = %e,
                            event = "reextraction_failed",
                            "re-extraction failed — doc stays pending"
                        );
                        None
                    }
                }
            }
            _ => None,
        };

        Ok(CompoundResult {
            ingest_stats,
            extraction_stats,
        })
    }

    /// Convert an absolute path to a repo-relative path.
    fn relative_path(&self, abs_path: &Path) -> String {
        if let Some(repo_root) = self.docs_root.parent().and_then(|p| p.parent()) {
            abs_path
                .strip_prefix(repo_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| abs_path.to_string_lossy().to_string())
        } else {
            abs_path.to_string_lossy().to_string()
        }
    }
}

/// Result of a compound ingest + extract operation.
#[derive(Debug)]
pub struct CompoundResult {
    pub ingest_stats: super::lexical_ingestor::DocStats,
    pub extraction_stats: Option<super::subject_extractor::ExtractionStats>,
}
