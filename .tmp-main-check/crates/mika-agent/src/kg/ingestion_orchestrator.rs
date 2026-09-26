//! # Ingestion Orchestrator
//!
//! Coordinates the lexical ingestor (#689), subject extractor (#690), and
//! entity resolver (#691). None of the modules have knowledge of each other's
//! internals — the orchestrator owns the sequencing.
//!
//! ## Responsibilities
//!
//! 1. **Compound hook** — after a doc is written, ingest chunks then extract
//!    subjects synchronously inline, then spawn async resolution.
//! 2. **Re-extraction on doc change** — four-phase capture → reingest →
//!    reconcile → re-resolve (D6 in #691 plan).
//!
//! ## Sole-Writer Separation
//!
//! - `LexicalIngestor` is the sole writer of `kg_chunks` and `search_content`.
//! - `SubjectExtractor` is the sole writer of `kg_subject_entities`,
//!   `kg_subject_relationships`, and provenance tables.
//! - `SubjectEntityResolver` is the sole writer of `kg_subject_resolutions`
//!   and `kg_resolutions_log`.
//! - This module calls all three but writes nothing itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use mika_common::llm::LlmProvider;

use crate::async_db::AsyncDatabase;

use super::entity_resolver::SubjectEntityResolver;
use super::lexical_ingestor::LexicalIngestor;
use super::subject_extractor::SubjectExtractor;

/// Coordinates lexical ingestion, subject extraction, and entity resolution.
///
/// Provides the compound hook entry point and re-extraction flow.
pub struct IngestionOrchestrator {
    db: AsyncDatabase,
    docs_root: PathBuf,
    /// LLM provider for extraction. `None` when extraction is disabled
    /// (no `MIKA_KG_EXTRACTION_MODEL` configured).
    extraction_llm: Option<Arc<dyn LlmProvider>>,
    /// LLM provider for entity resolution disambiguation. `None` when
    /// resolution LLM is disabled (exact-match-only mode).
    resolution_llm: Option<Arc<dyn LlmProvider>>,
    /// Per-batch LLM call budget for Stage-2 resolution (#757). The compound
    /// hook path is already bounded-by-construction (one doc's entities); the
    /// budget is defensive against a pathological doc that produces thousands
    /// of subject entities at once.
    resolution_budget: u32,
    trace_id: String,
    session_id: String,
    /// Precomputed `hash_docs_root(&docs_root)` — shared-corpus key for v27 KG tables.
    docs_root_hash: String,
}

impl IngestionOrchestrator {
    /// Create a new orchestrator.
    ///
    /// - `extraction_llm`: `None` to disable extraction (lexical ingestion
    ///   still runs).
    /// - `resolution_llm`: `None` for exact-match-only resolution mode.
    /// - `resolution_budget`: per-batch Stage-2 LLM call cap (#757). Callers
    ///   typically pass `settings.effective_kg_batch_budget()`.
    pub fn new(
        db: AsyncDatabase,
        docs_root: PathBuf,
        extraction_llm: Option<Arc<dyn LlmProvider>>,
        resolution_llm: Option<Arc<dyn LlmProvider>>,
        resolution_budget: u32,
        trace_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Self {
        let trace_id = trace_id
            .map(|s| s.to_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));
        let docs_root_hash = super::config::hash_docs_root(&docs_root);
        Self {
            db,
            docs_root,
            extraction_llm,
            resolution_llm,
            resolution_budget,
            trace_id,
            session_id: session_id.unwrap_or("compound").to_owned(),
            docs_root_hash,
        }
    }

    /// Compound hook: ingest a freshly-written document, then extract subjects,
    /// then spawn async resolution.
    ///
    /// Called after `/ce:compound` writes a doc. Lexical ingestion is synchronous
    /// and must succeed. Subject extraction is synchronous but failure is
    /// non-fatal (log-and-skip per C2.3) — the doc stays pending for next
    /// startup extraction. Entity resolution is spawned as a background task
    /// (D5 in #691 plan) — the authoring agent gets immediate subject entities
    /// and FTS5 queryability; resolutions appear shortly after.
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

            // Load roster for extraction-time constraint (#1158).
            // On load failure or non-Populated state, skip extraction entirely
            // (mirrors extract_pending's safety guard — never extract against
            // an untrusted roster).
            let roster = match extractor.load_roster_snapshot().await {
                Ok(r) if r.is_extractable() => r,
                Ok(r) => {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %rel_path,
                        load_state = ?r.load_state(),
                        event = "compound_roster_not_extractable",
                        "roster not extractable for compound extraction — skipping"
                    );
                    return Ok(CompoundResult {
                        ingest_stats,
                        extraction_stats: None,
                    });
                }
                Err(e) => {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %rel_path,
                        error = %e,
                        event = "compound_roster_load_failed",
                        "failed to load roster for compound extraction — skipping"
                    );
                    return Ok(CompoundResult {
                        ingest_stats,
                        extraction_stats: None,
                    });
                }
            };

            match extractor.extract_document(&rel_path, None, &roster).await {
                Ok(stats) => {
                    // extract_document writes the kg_extractions marker
                    // atomically with the extraction results (#757 review P1),
                    // so no separate record_extraction call is needed here.

                    // 3. Spawn async resolution for extracted entities (D5 in #691).
                    // The compound write returns immediately; resolutions appear shortly after.
                    if stats.entities_upserted > 0 {
                        self.spawn_resolution(&rel_path);
                    }

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

    /// Re-extraction on doc change: four-phase capture → reingest → reconcile →
    /// re-resolve (D6 in #691 plan).
    ///
    /// Phase 1: Capture previous provenance state.
    /// Phase 2: Re-ingest chunks (handled by `LexicalIngestor`).
    /// Phase 3: Re-extract with previous state for orphan sweep.
    /// Phase 4: Re-resolve entities touched by re-extraction.
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

                // Load roster for extraction-time constraint (#1158).
                // On load failure or non-Populated state, skip re-extraction
                // (mirrors extract_pending's safety guard).
                let roster = match extractor.load_roster_snapshot().await {
                    Ok(r) if r.is_extractable() => r,
                    Ok(r) => {
                        warn!(
                            trace_id = %self.trace_id,
                            doc = %rel_path,
                            load_state = ?r.load_state(),
                            event = "compound_roster_not_extractable",
                            "roster not extractable for reingest extraction — skipping"
                        );
                        return Ok(CompoundResult {
                            ingest_stats,
                            extraction_stats: None,
                        });
                    }
                    Err(e) => {
                        warn!(
                            trace_id = %self.trace_id,
                            doc = %rel_path,
                            error = %e,
                            event = "compound_roster_load_failed",
                            "failed to load roster for reingest extraction — skipping"
                        );
                        return Ok(CompoundResult {
                            ingest_stats,
                            extraction_stats: None,
                        });
                    }
                };

                match extractor
                    .extract_document(&rel_path, Some(prev), &roster)
                    .await
                {
                    Ok(stats) => {
                        // extract_document writes the kg_extractions marker
                        // atomically with the extraction results (#757 review P1).

                        // Phase 4: Re-resolve entities touched by re-extraction (D6).
                        // Coarse re-resolution — all entities from this doc re-resolve.
                        // Failures: C2.3 log-and-skip, pending for next startup.
                        if stats.entities_upserted > 0 {
                            self.spawn_resolution(&rel_path);
                        }

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

    /// Spawn background resolution for entities from a doc.
    ///
    /// Resolution runs as a background task so the compound write returns
    /// immediately. Failures are logged per C2.3 — entities stay pending
    /// for next startup's `resolve_pending`.
    fn spawn_resolution(&self, doc_path: &str) {
        let db = self.db.clone();
        let resolution_llm = self.resolution_llm.clone();
        let trace_id = self.trace_id.clone();
        let agent_id = self.db.agent_id.clone();
        let doc_path = doc_path.to_owned();
        let budget = self.resolution_budget;
        let docs_root_hash = self.docs_root_hash.clone();

        tokio::spawn(async move {
            let resolver = SubjectEntityResolver::new(
                db,
                resolution_llm,
                vec![docs_root_hash],
                Some(&trace_id),
            );

            match resolver.resolve_pending(budget).await {
                Ok(stats) => {
                    if stats.matched_exact > 0 || stats.matched_llm > 0 || stats.errors > 0 {
                        info!(
                            agent_id = %agent_id,
                            doc = %doc_path,
                            matched_exact = stats.matched_exact,
                            matched_llm = stats.matched_llm,
                            no_match = stats.no_match,
                            errors = stats.errors,
                            event = "compound_resolution_complete",
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        agent_id = %agent_id,
                        doc = %doc_path,
                        error = %e,
                        event = "compound_resolution_failed",
                        "resolution failed — entities stay pending per C2.3"
                    );
                }
            }
        });
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
