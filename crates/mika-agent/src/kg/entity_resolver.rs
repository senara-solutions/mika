//! # Subject Entity Resolver
//!
//! Resolves extracted subject entities to domain graph nodes (#691).
//! Two-stage pipeline: exact match (case-insensitive) then LLM disambiguation
//! for unresolved or ambiguous cases.
//!
//! ## Sole-Writer Contract
//!
//! This module is the sole writer of `kg_subject_resolutions` and
//! `kg_resolutions_log` rows. No other code path writes these tables.
//!
//! This module also reads and deletes from `kg_invalidated_no_match` (#961)
//! to track reattempted entities whose prior `no_match` log rows were
//! invalidated by domain-graph rebuild (#960).
//!
//! ## Resolution Pipeline
//!
//! For each subject entity with a well-known type (skill, tool, agent, problem_type, concept):
//!
//! 1. **Exact match:** Case-insensitive match against `kg_entities.entity_key`.
//!    If match found and extraction confidence > 0.9, resolve immediately.
//! 2. **LLM disambiguation:** For unmatched or low-confidence entities, send to
//!    the resolution LLM with candidate list and source chunk context.
//! 3. **Tracking:** Every resolution attempt writes a `kg_resolutions_log` row.
//!
//! Discovered types (solution_path, failure_mode, pattern) skip resolution
//! entirely — no domain counterpart exists (D8).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use mika_common::llm::{LlmMessage, LlmProvider, LlmRequest, LlmRole, LlmUsage};

use crate::async_db::AsyncDatabase;
use crate::db::kg_schema::KG_DOMAIN_ENTITY_TYPES;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Confidence threshold for exact-match shortcircuit (D1 step 2).
/// Entities with extraction confidence above this skip LLM disambiguation.
const EXACT_MATCH_CONFIDENCE_THRESHOLD: f64 = 0.9;

/// Maximum number of domain candidates to include in a disambiguation prompt (D3).
const MAX_DISAMBIGUATION_CANDIDATES: usize = 50;

/// Session ID sentinel for audit events and llm_calls emitted during resolution.
const RESOLUTION_SESSION_ID: &str = "resolution";

/// Resolution outcome values matching the CHECK constraint on `kg_resolutions_log.outcome`.
mod outcome {
    pub const MATCHED_EXACT: &str = "matched_exact";
    pub const MATCHED_LLM: &str = "matched_llm";
    pub const MATCHED_LLM_DB_FALLBACK: &str = "matched_llm_db_fallback";
    pub const NO_MATCH: &str = "no_match";
    pub const NO_CANDIDATE_OF_TYPE: &str = "no_candidate_of_type";
    pub const SKIPPED_DISCOVERED_TYPE: &str = "skipped_discovered_type";
    pub const SKIPPED_NO_LLM: &str = "skipped_no_llm";
    pub const ERROR: &str = "error";
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A subject entity pending resolution.
#[derive(Debug, Clone)]
struct PendingEntity {
    id: i64,
    entity_key: String,
    entity_type: String,
    #[allow(dead_code)] // Loaded from DB schema; may be used in future diagnostics.
    name: String,
    confidence: f64,
    /// The corpus this entity belongs to (shared-layer key from v27).
    docs_root_hash: String,
    /// True when this entity has a marker in `kg_invalidated_no_match` (#961),
    /// meaning its prior `no_match` resolution log row was deleted by
    /// domain-graph rebuild invalidation (#960).
    was_invalidated: bool,
}

/// A domain entity candidate for disambiguation.
#[derive(Debug, Clone)]
struct DomainCandidate {
    id: i64,
    entity_key: String,
    properties_json: Option<String>,
}

/// LLM disambiguation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DisambiguationResponse {
    #[serde(rename = "match")]
    matched: Option<String>,
    confidence: f64,
}

/// Result of resolving a single entity.
#[derive(Debug, Clone)]
enum ResolutionResult {
    /// Exact match found with given domain entity ID and confidence.
    ExactMatch {
        domain_entity_id: i64,
        confidence: f64,
    },
    /// LLM picked a match (in-prompt candidate list).
    LlmMatch {
        domain_entity_id: i64,
        confidence: f64,
    },
    /// LLM picked a match not in the in-prompt candidate list, but validated
    /// against `kg_entities` via DB fallback lookup (#874).
    LlmMatchDbFallback {
        domain_entity_id: i64,
        confidence: f64,
    },
    /// LLM said no match (candidates existed but none matched).
    NoMatch,
    /// No domain entity of this subject's type exists — extractor produced a
    /// phantom subject with no possible domain counterpart (#1154).
    NoCandidateOfType,
    /// Discovered type — no domain counterpart.
    SkippedDiscoveredType,
    /// No LLM configured and exact match failed.
    SkippedNoLlm,
    /// Per-batch LLM budget exhausted — entity stays pending for next run (#757).
    SkippedBudget,
    /// Error during resolution.
    Error(String),
}

/// Statistics from a resolution pass (used by both per-doc and batch resolution).
#[derive(Debug, Clone, Default)]
pub struct ResolutionStats {
    pub total: usize,
    pub matched_exact: usize,
    pub matched_llm: usize,
    pub matched_llm_db_fallback: usize,
    pub no_match: usize,
    pub no_candidate_of_type: usize,
    pub skipped_discovered: usize,
    pub skipped_no_llm: usize,
    pub errors: usize,
    pub duration_ms: u64,
    /// True when the batch stopped early because the LLM call budget was
    /// exhausted (#757). Remaining entities stay pending for the next run.
    pub aborted_budget: bool,
    /// Number of LLM disambiguation calls made. Stage 1 exact matches do NOT
    /// debit this counter — only Stage 2 LLM calls.
    pub llm_calls: u32,
    /// Per-corpus entity attempt counts for fairness observability (#927).
    pub per_corpus_attempted: HashMap<String, u32>,
    /// Count of entities this batch that were re-attempts from prior `no_match`
    /// outcomes invalidated by domain-graph rebuild (#961). Zero when no
    /// invalidation occurred; non-zero on the first resolution pass after a
    /// rebuild that invalidated rows for an extant type.
    pub reattempted_no_match: u32,
}

// ---------------------------------------------------------------------------
// SubjectEntityResolver (Units 2, 3, 4)
// ---------------------------------------------------------------------------

/// Sole writer of `kg_subject_resolutions` and `kg_resolutions_log`.
///
/// Invariants:
/// - Reads from both `kg_subject_entities` and `kg_entities`.
/// - Writes only to `kg_subject_resolutions` and `kg_resolutions_log`.
/// - Does not modify `kg_entities`, `kg_subject_entities`, or any other table.
/// - All disambiguation calls use `MIKA_KG_RESOLUTION_MODEL` (per C2.1).
pub struct SubjectEntityResolver {
    db: AsyncDatabase,
    /// `None` when no resolution model configured — exact-match-only mode.
    llm: Option<Arc<dyn LlmProvider>>,
    trace_id: String,
    /// Shared-corpus keys for querying v27 shared-layer tables (#798: multi-corpus).
    /// Reads from `kg_subject_entities` and `kg_chunk_subjects` use these
    /// instead of `agent_id`. Writes to `kg_subject_resolutions` and
    /// `kg_resolutions_log` still use `agent_id` (per-agent).
    docs_root_hashes: Vec<String>,
}

impl SubjectEntityResolver {
    /// Create a new resolver.
    ///
    /// - `db`: async database handle (carries `agent_id` implicitly).
    /// - `llm`: LLM provider for disambiguation. `None` for exact-match-only mode.
    /// - `docs_root_hashes`: shared-corpus keys for v27 shared-layer table reads (#798).
    ///   Single-corpus agents pass a one-element vec.
    /// - `trace_id`: optional trace ID for observability.
    pub fn new(
        db: AsyncDatabase,
        llm: Option<Arc<dyn LlmProvider>>,
        docs_root_hashes: Vec<String>,
        trace_id: Option<&str>,
    ) -> Self {
        let trace_id = trace_id
            .map(|s| s.to_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));

        if llm.is_none() {
            warn!(
                trace_id = %trace_id,
                event = "resolution_llm_disabled",
                "no resolution model configured — exact-match-only mode"
            );
        }

        Self {
            db,
            llm,
            trace_id,
            docs_root_hashes,
        }
    }

    /// Resolve entities produced by a single doc's extraction (D5).
    ///
    /// Called as per-doc follow-on after `extract_document()`.
    /// `entity_ids` are the IDs of subject entities to resolve.
    /// `extraction_trace_id` is recorded for staleness tracking.
    ///
    /// `budget` caps Stage 2 LLM disambiguation calls (#757). The compound
    /// hook path is bounded-by-construction (one doc's entities), so callers
    /// typically pass the configured budget; the cap still defends against a
    /// pathological single doc that produces thousands of entities.
    pub async fn resolve_doc_entities(
        &self,
        entity_ids: &[i64],
        extraction_trace_id: &str,
        budget: u32,
    ) -> Result<ResolutionStats> {
        if entity_ids.is_empty() {
            return Ok(ResolutionStats::default());
        }

        let entities = self.get_entities_by_ids(entity_ids).await?;
        let trace_lookup = |_id: i64| extraction_trace_id.to_owned();
        let stats = self
            .resolve_entities(&entities, &trace_lookup, "resolution_doc_complete", budget)
            .await;

        // Emit audit event for the per-doc batch.
        self.emit_audit_event(&stats).await;

        Ok(stats)
    }

    /// Resolve all pending entities for an agent (startup or re-resolution),
    /// capped by `budget` Stage-2 LLM disambiguation calls (#757 R2).
    ///
    /// `budget == 0` short-circuits with no LLM calls. On overflow the method
    /// aborts cleanly, emits a `kg_budget_exhausted` WARN, and leaves
    /// remaining entities pending for the next run. Stage-1 exact matches do
    /// not consume the budget, so entities that resolve without the LLM still
    /// make progress even when `budget` is tight.
    pub async fn resolve_pending(&self, budget: u32) -> Result<ResolutionStats> {
        let start = Instant::now();
        let agent_id = self.db.agent_id.clone();

        let pending = self.get_pending_entities(budget).await?;

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            pending = pending.len(),
            budget = budget,
            event = "resolution_pending_start",
        );

        if pending.is_empty() {
            return Ok(ResolutionStats {
                duration_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            });
        }

        // Get the latest extraction trace_id for each entity.
        let entity_ids: Vec<i64> = pending.iter().map(|e| e.id).collect();
        let trace_map = self.get_extraction_trace_ids(&entity_ids).await?;
        let fallback_trace = self.trace_id.clone();
        let trace_lookup = |id: i64| {
            trace_map
                .get(&id)
                .cloned()
                .unwrap_or_else(|| fallback_trace.clone())
        };

        let mut stats = self
            .resolve_entities(
                &pending,
                &trace_lookup,
                "resolution_pending_complete",
                budget,
            )
            .await;
        stats.duration_ms = start.elapsed().as_millis() as u64;

        Ok(stats)
    }

    /// Shared resolution loop for both per-doc and batch paths.
    ///
    /// `trace_lookup` maps entity ID → extraction trace ID for staleness tracking.
    /// `budget` caps Stage-2 LLM disambiguation calls (#757). Stage-1 exact
    /// matches do not debit the budget. When the budget is hit, remaining
    /// entities that would need Stage-2 are left unprocessed — they stay
    /// pending via the absence of a `kg_resolutions_log` row — and
    /// `aborted_budget` is set on the returned stats.
    async fn resolve_entities<F>(
        &self,
        entities: &[PendingEntity],
        trace_lookup: &F,
        completion_event: &str,
        budget: u32,
    ) -> ResolutionStats
    where
        F: Fn(i64) -> String,
    {
        let agent_id = self.db.agent_id.clone();
        let mut stats = ResolutionStats {
            total: entities.len(),
            ..Default::default()
        };

        for (i, entity) in entities.iter().enumerate() {
            // Per-corpus fairness tracking (#927).
            *stats
                .per_corpus_attempted
                .entry(entity.docs_root_hash.clone())
                .or_insert(0) += 1;

            // Stage-2 budget guard: would we call the LLM if needed?
            // `llm_call_allowed = false` tells resolve_single_entity to
            // short-circuit before a Stage-2 call with `SkippedBudget`.
            // Stage-1 exact matches still process for free even when the
            // budget is exhausted — they cost no LLM calls, so skipping
            // remaining entities when one needs Stage-2 would defer free
            // work (#757 review finding P1).
            let llm_call_allowed = stats.llm_calls < budget;

            let extraction_trace_id = trace_lookup(entity.id);
            let entity_start = Instant::now();
            let (result, used_llm) = self.resolve_single_entity(entity, llm_call_allowed).await;
            let duration_ms = entity_start.elapsed().as_millis() as i64;

            if used_llm {
                stats.llm_calls = stats.llm_calls.saturating_add(1);
            }

            // If the budget prevented a Stage-2 call, record the abort once
            // and continue — remaining entities may still resolve via Stage-1
            // exact match for free. Entities that return SkippedBudget leave
            // no `kg_resolutions_log` row, so they stay pending for next run.
            if matches!(result, ResolutionResult::SkippedBudget) {
                if !stats.aborted_budget {
                    stats.aborted_budget = true;
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        scope = "resolution",
                        calls_made = stats.llm_calls,
                        budget = budget,
                        remaining = entities.len() - i,
                        event = "kg_budget_exhausted",
                        "resolution hit per-batch LLM call cap — entities needing LLM disambiguation stay pending; Stage-1 exact matches still proceed"
                    );
                }
                continue;
            }

            self.apply_result(
                entity,
                &result,
                &extraction_trace_id,
                duration_ms,
                &mut stats,
            )
            .await;

            // Track reattempted no_match entities (#961). SkippedBudget
            // entities never reach here (the `continue` above skips them);
            // all remaining outcomes wrote a log row via apply_result.
            if entity.was_invalidated {
                stats.reattempted_no_match += 1;
            }

            if let ResolutionResult::Error(msg) = &result {
                warn!(
                    trace_id = %self.trace_id,
                    agent_id = %agent_id,
                    entity_key = %entity.entity_key,
                    error = %msg,
                    event = "resolution_entity_error",
                );
            }
        }

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            total = stats.total,
            matched_exact = stats.matched_exact,
            matched_llm = stats.matched_llm,
            matched_llm_db_fallback = stats.matched_llm_db_fallback,
            no_match = stats.no_match,
            no_candidate_of_type = stats.no_candidate_of_type,
            skipped_discovered = stats.skipped_discovered,
            skipped_no_llm = stats.skipped_no_llm,
            errors = stats.errors,
            llm_calls = stats.llm_calls,
            aborted_budget = stats.aborted_budget,
            reattempted_no_match = stats.reattempted_no_match,
            event = %completion_event,
        );

        stats
    }

    /// Write resolution and log rows for a single entity result, update stats.
    async fn apply_result(
        &self,
        entity: &PendingEntity,
        result: &ResolutionResult,
        extraction_trace_id: &str,
        duration_ms: i64,
        stats: &mut ResolutionStats,
    ) {
        match result {
            ResolutionResult::ExactMatch {
                domain_entity_id,
                confidence,
            } => {
                self.write_resolution(entity.id, *domain_entity_id, *confidence)
                    .await;
                self.write_log(
                    entity.id,
                    outcome::MATCHED_EXACT,
                    extraction_trace_id,
                    None,
                    Some(duration_ms),
                )
                .await;
                stats.matched_exact += 1;
            }
            ResolutionResult::LlmMatch {
                domain_entity_id,
                confidence,
            } => {
                self.write_resolution(entity.id, *domain_entity_id, *confidence)
                    .await;
                let model = self.llm.as_ref().map(|l| l.model_name().to_string());
                self.write_log(
                    entity.id,
                    outcome::MATCHED_LLM,
                    extraction_trace_id,
                    model.as_deref(),
                    Some(duration_ms),
                )
                .await;
                stats.matched_llm += 1;
            }
            ResolutionResult::LlmMatchDbFallback {
                domain_entity_id,
                confidence,
            } => {
                self.write_resolution(entity.id, *domain_entity_id, *confidence)
                    .await;
                let model = self.llm.as_ref().map(|l| l.model_name().to_string());
                self.write_log(
                    entity.id,
                    outcome::MATCHED_LLM_DB_FALLBACK,
                    extraction_trace_id,
                    model.as_deref(),
                    Some(duration_ms),
                )
                .await;
                stats.matched_llm_db_fallback += 1;
            }
            ResolutionResult::NoMatch => {
                let model = self.llm.as_ref().map(|l| l.model_name().to_string());
                self.write_log(
                    entity.id,
                    outcome::NO_MATCH,
                    extraction_trace_id,
                    model.as_deref(),
                    Some(duration_ms),
                )
                .await;
                stats.no_match += 1;
            }
            ResolutionResult::NoCandidateOfType => {
                let model = self.llm.as_ref().map(|l| l.model_name().to_string());
                self.write_log(
                    entity.id,
                    outcome::NO_CANDIDATE_OF_TYPE,
                    extraction_trace_id,
                    model.as_deref(),
                    Some(duration_ms),
                )
                .await;
                stats.no_candidate_of_type += 1;
            }
            ResolutionResult::SkippedDiscoveredType => {
                self.write_log(
                    entity.id,
                    outcome::SKIPPED_DISCOVERED_TYPE,
                    extraction_trace_id,
                    None,
                    Some(duration_ms),
                )
                .await;
                stats.skipped_discovered += 1;
            }
            ResolutionResult::SkippedNoLlm => {
                self.write_log(
                    entity.id,
                    outcome::SKIPPED_NO_LLM,
                    extraction_trace_id,
                    None,
                    Some(duration_ms),
                )
                .await;
                stats.skipped_no_llm += 1;
            }
            ResolutionResult::SkippedBudget => {
                // Entity stays pending — do NOT write a kg_resolutions_log row.
                // The outer loop breaks as soon as it sees this variant and
                // records aborted_budget=true in ResolutionStats (#757).
            }
            ResolutionResult::Error(_) => {
                self.write_log(
                    entity.id,
                    outcome::ERROR,
                    extraction_trace_id,
                    None,
                    Some(duration_ms),
                )
                .await;
                stats.errors += 1;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Core resolution logic (Units 2 + 3)
    // -----------------------------------------------------------------------

    /// Resolve a single entity through the two-stage pipeline.
    ///
    /// Returns `(result, used_llm)`. `used_llm` is `true` iff a Stage-2 LLM
    /// disambiguation call was actually issued. Stage-1 exact matches and all
    /// early returns (discovered type, no-LLM mode, DB error) report `false`
    /// so the caller can debit the per-batch budget correctly (#757).
    ///
    /// When `llm_call_allowed = false`, the function short-circuits before a
    /// Stage-2 call and returns `SkippedBudget`. The caller treats this as
    /// "entity stays pending; abort the batch." Stage-1 exact matches still
    /// proceed under this flag — they cost nothing.
    async fn resolve_single_entity(
        &self,
        entity: &PendingEntity,
        llm_call_allowed: bool,
    ) -> (ResolutionResult, bool) {
        // D8: Discovered types skip resolution entirely.
        if !KG_DOMAIN_ENTITY_TYPES.contains(&entity.entity_type.as_str()) {
            return (ResolutionResult::SkippedDiscoveredType, false);
        }

        // Stage 1: Exact match (D1).
        // Track whether Stage 1 found an existing domain entity (even if
        // low-confidence) vs. no entity_key match at all. This distinction
        // drives the NoMatch vs NoCandidateOfType outcome split (#1154).
        let stage1_found = match self.try_exact_match(entity).await {
            Ok(Some(domain)) => {
                // Exact match found. If extraction confidence > threshold, resolve.
                if entity.confidence > EXACT_MATCH_CONFIDENCE_THRESHOLD {
                    return (
                        ResolutionResult::ExactMatch {
                            domain_entity_id: domain.id,
                            confidence: entity.confidence,
                        },
                        false,
                    );
                }
                // Low confidence — escalate to LLM for verification.
                true
            }
            Ok(None) => {
                // No exact match — escalate to LLM.
                false
            }
            Err(e) => {
                return (
                    ResolutionResult::Error(format!("exact match query failed: {e}")),
                    false,
                );
            }
        };

        // Stage 2: LLM disambiguation (D1 stage 2).
        let Some(ref llm) = self.llm else {
            return (ResolutionResult::SkippedNoLlm, false);
        };

        // #757: respect per-batch budget before issuing the LLM call.
        if !llm_call_allowed {
            return (ResolutionResult::SkippedBudget, false);
        }

        // #1154: Pre-check for empty candidates before entering the LLM path.
        // If no domain entities of this type exist at all, the LLM cannot
        // disambiguate — short-circuit to NoCandidateOfType without an LLM call.
        let candidates = match self.get_domain_candidates(&entity.entity_type).await {
            Ok(c) => c,
            Err(e) => {
                return (
                    ResolutionResult::Error(format!("candidate fetch failed: {e}")),
                    false,
                );
            }
        };
        if candidates.is_empty() {
            return (ResolutionResult::NoCandidateOfType, false);
        }

        match self
            .disambiguate_with_llm_candidates(llm, entity, candidates)
            .await
        {
            Ok(Some((domain_id, llm_confidence, db_fallback))) => {
                // D2: confidence = min(extraction_confidence, llm_confidence).
                let combined_confidence = entity.confidence.min(llm_confidence);
                if db_fallback {
                    (
                        ResolutionResult::LlmMatchDbFallback {
                            domain_entity_id: domain_id,
                            confidence: combined_confidence,
                        },
                        true,
                    )
                } else {
                    (
                        ResolutionResult::LlmMatch {
                            domain_entity_id: domain_id,
                            confidence: combined_confidence,
                        },
                        true,
                    )
                }
            }
            // #1154: Split no-match outcome based on Stage 1 result.
            // If Stage 1 found an exact entity_key match (low confidence),
            // candidates existed but disambiguation failed → NoMatch.
            // If Stage 1 found nothing, the extractor produced a phantom
            // subject with no domain counterpart → NoCandidateOfType.
            Ok(None) => {
                if stage1_found {
                    (ResolutionResult::NoMatch, true)
                } else {
                    (ResolutionResult::NoCandidateOfType, true)
                }
            }
            Err(e) => (
                ResolutionResult::Error(format!("LLM disambiguation failed: {e}")),
                true,
            ),
        }
    }

    /// Stage 1: Case-insensitive exact match against `kg_entities`.
    async fn try_exact_match(&self, entity: &PendingEntity) -> Result<Option<DomainCandidate>> {
        let entity_key = entity.entity_key.clone();

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT id, entity_key, properties_json FROM kg_entities
                     WHERE LOWER(entity_key) = LOWER(?1)",
                )?;
                let candidates: Vec<DomainCandidate> = stmt
                    .query_map(rusqlite::params![entity_key], |row| {
                        Ok(DomainCandidate {
                            id: row.get(0)?,
                            entity_key: row.get(1)?,
                            properties_json: row.get(2)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                // Exactly one match → return it.
                if candidates.len() == 1 {
                    Ok(Some(candidates.into_iter().next().unwrap()))
                } else {
                    // No match or multiple matches (shouldn't happen with UNIQUE constraint)
                    Ok(None)
                }
            })
            .await
    }

    /// Stage 2: LLM disambiguation with chunk context and candidate list.
    ///
    /// Returns `(domain_entity_id, confidence, db_fallback)`. `db_fallback` is
    /// `true` when the LLM's `matched_key` was not in the in-prompt candidate
    /// slice but was validated against `kg_entities` via DB lookup (#874).
    ///
    /// Accepts pre-fetched candidates (#1154). The caller fetches candidates
    /// and handles the empty-candidates case before entering this function.
    async fn disambiguate_with_llm_candidates(
        &self,
        llm: &Arc<dyn LlmProvider>,
        entity: &PendingEntity,
        candidates: Vec<DomainCandidate>,
    ) -> Result<Option<(i64, f64, bool)>> {
        let agent_id = self.db.agent_id.clone();

        // 2. Fetch source chunk context.
        let chunk_context = self.get_chunk_context(entity.id).await?;

        // 3. Build disambiguation prompt.
        let (system, user) = build_disambiguation_prompt(entity, &candidates, &chunk_context);

        let request = LlmRequest {
            model: llm.model_name().to_string(),
            system: Some(system),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: mika_common::llm::LlmContent::Text(user),
            }],
            tools: None,
            max_tokens: llm.max_tokens(),
            thinking: None,
        };

        // 4. LLM call with C2.2 retry taxonomy.
        let start = Instant::now();
        let (response_text, first_usage) = match self.call_llm_with_retry(llm, &request).await? {
            Some(pair) => pair,
            None => return Ok(None), // All retries exhausted, log-and-skip.
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        // 5. Parse and validate response. Semantic retry on malformed JSON (C2.2).
        let parsed = match parse_disambiguation_json(&response_text) {
            Ok(p) => {
                // Store llm_calls row for the successful first call (C2.4).
                self.store_llm_call(
                    &entity.entity_key,
                    latency_ms,
                    llm.provider_name(),
                    llm.model_name(),
                    &first_usage,
                    "kg_resolution",
                )
                .await;
                p
            }
            Err(parse_err) => {
                warn!(
                    trace_id = %self.trace_id,
                    entity_key = %entity.entity_key,
                    error = %parse_err,
                    event = "resolution_parse_failed_retry",
                    "malformed JSON from LLM — retrying with reinforcement"
                );
                // Store llm_calls row for the failed first call before retry.
                self.store_llm_call(
                    &entity.entity_key,
                    latency_ms,
                    llm.provider_name(),
                    llm.model_name(),
                    &first_usage,
                    "kg_resolution",
                )
                .await;
                let retry_start = Instant::now();
                match self
                    .retry_disambiguation_with_reinforcement(llm, &request, &response_text)
                    .await
                {
                    Ok(Some((p, retry_usage))) => {
                        let retry_latency = retry_start.elapsed().as_millis() as u64;
                        // Store separate llm_calls row for the retry call.
                        self.store_llm_call(
                            &entity.entity_key,
                            retry_latency,
                            llm.provider_name(),
                            llm.model_name(),
                            &retry_usage,
                            "kg_resolution_retry",
                        )
                        .await;
                        p
                    }
                    Ok(None) | Err(_) => return Ok(None), // Log-and-skip per C2.3.
                }
            }
        };

        match parsed.matched {
            Some(matched_key) => {
                // Validate confidence range first (shared across all accept paths).
                if !(0.0..=1.0).contains(&parsed.confidence) {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        entity_key = %entity.entity_key,
                        confidence = parsed.confidence,
                        event = "resolution_confidence_out_of_range",
                    );
                    return Ok(None);
                }

                // Path 1: matched_key in the in-prompt candidate list.
                if let Some(candidate) = candidates
                    .iter()
                    .find(|c| c.entity_key.eq_ignore_ascii_case(&matched_key))
                {
                    return Ok(Some((candidate.id, parsed.confidence, false)));
                }

                // Path 2–4: matched_key NOT in the in-prompt slice — DB fallback (#874).
                // Type-bounded lookup against kg_entities.
                match self
                    .try_domain_entity_by_key(&entity.entity_type, &matched_key)
                    .await
                {
                    Ok(Some(domain_candidate)) => {
                        // Path 2: DB-fallback hit, same type → accept as matched_llm_db_fallback.
                        info!(
                            trace_id = %self.trace_id,
                            agent_id = %agent_id,
                            entity_key = %entity.entity_key,
                            entity_type = %entity.entity_type,
                            matched_key = %matched_key,
                            domain_entity_id = domain_candidate.id,
                            event = "resolution_matched_key_db_fallback_hit",
                        );
                        Ok(Some((domain_candidate.id, parsed.confidence, true)))
                    }
                    Ok(None) => {
                        // matched_key not found under same type — check if it exists
                        // under a different type (Path 3) or not at all (Path 4).
                        match self.try_domain_entity_any_type(&matched_key).await {
                            Ok(Some(found_entity_key)) => {
                                // Path 3: cross-type match → reject with diagnostic.
                                let found_type =
                                    found_entity_key.split(':').next().unwrap_or("unknown");
                                warn!(
                                    trace_id = %self.trace_id,
                                    agent_id = %agent_id,
                                    entity_key = %entity.entity_key,
                                    entity_type = %entity.entity_type,
                                    matched_key = %matched_key,
                                    found_type = %found_type,
                                    event = "resolution_matched_key_cross_type_rejected",
                                );
                            }
                            Ok(None) => {
                                // Path 4: not in kg_entities at all → no_match.
                                warn!(
                                    trace_id = %self.trace_id,
                                    agent_id = %agent_id,
                                    entity_key = %entity.entity_key,
                                    entity_type = %entity.entity_type,
                                    matched_key = %matched_key,
                                    db_fallback_attempted = true,
                                    db_fallback_hit = false,
                                    event = "resolution_matched_key_not_in_candidates",
                                    "LLM returned entity_key not in candidate list or kg_entities — treating as no_match"
                                );
                            }
                            Err(e) => {
                                // Diagnostic lookup failed — fall through to no_match.
                                warn!(
                                    trace_id = %self.trace_id,
                                    agent_id = %agent_id,
                                    entity_key = %entity.entity_key,
                                    matched_key = %matched_key,
                                    error = %e,
                                    event = "resolution_matched_key_not_in_candidates",
                                    "LLM returned entity_key not in candidate list (diagnostic lookup failed) — treating as no_match"
                                );
                            }
                        }
                        Ok(None)
                    }
                    Err(e) => {
                        // DB fallback lookup failed — treat as no_match (fail-open for diagnosis).
                        warn!(
                            trace_id = %self.trace_id,
                            agent_id = %agent_id,
                            entity_key = %entity.entity_key,
                            matched_key = %matched_key,
                            error = %e,
                            db_fallback_attempted = true,
                            db_fallback_hit = false,
                            event = "resolution_matched_key_not_in_candidates",
                            "DB fallback lookup failed — treating as no_match"
                        );
                        Ok(None)
                    }
                }
            }
            None => Ok(None), // LLM said no match.
        }
    }

    // -----------------------------------------------------------------------
    // DB helpers
    // -----------------------------------------------------------------------

    /// Fetch subject entities by IDs.
    async fn get_entities_by_ids(&self, ids: &[i64]) -> Result<Vec<PendingEntity>> {
        let ids = ids.to_vec();
        let docs_root_hashes = self.docs_root_hashes.clone();

        self.db
            .with_db(move |db| {
                if docs_root_hashes.is_empty() {
                    return Ok(Vec::new());
                }
                let hash_placeholders: Vec<String> =
                    docs_root_hashes.iter().map(|_| "?".to_string()).collect();
                let id_placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT id, entity_key, type, name, confidence, docs_root_hash
                     FROM kg_subject_entities
                     WHERE docs_root_hash IN ({}) AND id IN ({})",
                    hash_placeholders.join(","),
                    id_placeholders.join(",")
                );
                let mut stmt = db.conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                for h in &docs_root_hashes {
                    params.push(Box::new(h.clone()));
                }
                for id in &ids {
                    params.push(Box::new(*id));
                }
                let entities: Vec<PendingEntity> = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        Ok(PendingEntity {
                            id: row.get(0)?,
                            entity_key: row.get(1)?,
                            entity_type: row.get(2)?,
                            name: row.get(3)?,
                            confidence: row.get(4)?,
                            docs_root_hash: row.get(5)?,
                            // Compound-hook path resolves freshly extracted
                            // entities, not invalidation reattempts (#961).
                            was_invalidated: false,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(entities)
            })
            .await
    }

    /// Count pending entities without fetching full rows (#906).
    ///
    /// Mirrors `get_pending_entities`'s WHERE clause (same `docs_root_hash
    /// IN (?,?,...)` multi-corpus pattern, same `LEFT JOIN kg_resolutions_log`
    /// pending semantics) but with `SELECT COUNT(*)` projection.
    ///
    /// Used by the periodic resolver tick for observability (pending_before /
    /// pending_after logging). Cheap — one SQLite query, no row materialization.
    pub async fn count_pending(&self) -> Result<u64> {
        let agent_id = self.db.agent_id.clone();
        let docs_root_hashes = self.docs_root_hashes.clone();

        self.db
            .with_db(move |db| {
                if docs_root_hashes.is_empty() {
                    return Ok(0);
                }
                let hash_placeholders: Vec<String> =
                    docs_root_hashes.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT COUNT(*)
                     FROM kg_subject_entities e
                     LEFT JOIN kg_resolutions_log r
                         ON r.subject_entity_id = e.id AND r.agent_id = ?
                     WHERE e.docs_root_hash IN ({})
                       AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
                       AND (
                         r.id IS NULL
                         OR r.source_extraction_trace_id != (
                             SELECT cs.extraction_trace_id
                             FROM kg_chunk_subjects cs
                             WHERE cs.subject_entity_id = e.id
                             ORDER BY cs.created_at DESC LIMIT 1
                         )
                       )",
                    hash_placeholders.join(","),
                );
                let mut stmt = db.conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id)];
                for h in &docs_root_hashes {
                    params.push(Box::new(h.clone()));
                }
                let count: u64 =
                    stmt.query_row(rusqlite::params_from_iter(params), |row| row.get(0))?;
                Ok(count)
            })
            .await
    }

    /// Get pending entities with per-corpus fairness (#927).
    ///
    /// Uses single-pass reallocation (plan F2) to distribute `total_budget`
    /// across corpora, then round-robin interleaves results (KTD-3) so no
    /// single large corpus starves smaller ones.
    ///
    /// For single-corpus agents this degenerates to one
    /// `get_pending_entities_for_corpus` call.
    async fn get_pending_entities(&self, total_budget: u32) -> Result<Vec<PendingEntity>> {
        if self.docs_root_hashes.is_empty() {
            return Ok(Vec::new());
        }

        // When budget is 0, Stage-2 LLM calls are blocked but Stage-1 exact
        // matches still proceed for free (documented invariant in CLAUDE.md).
        // Use a minimal selection limit so we don't load the entire backlog.
        let effective_budget = if total_budget == 0 { 50 } else { total_budget };

        // Single-corpus fast path — skip allocation overhead.
        if self.docs_root_hashes.len() == 1 {
            let limit = (effective_budget as usize * 2).max(50) as u32;
            return self
                .get_pending_entities_for_corpus(&self.docs_root_hashes[0], limit)
                .await;
        }

        // 1. Query each corpus's pending count.
        let mut corpus_counts: Vec<(String, u32)> = Vec::new();
        for hash in &self.docs_root_hashes {
            let count = self.count_pending_for_corpus(hash).await?;
            corpus_counts.push((hash.clone(), count));
        }

        // 2-4. Fair budget allocation via shared function (#927, #962).
        let pending_counts: Vec<u32> = corpus_counts.iter().map(|(_, count)| *count).collect();
        let assigned = super::budget::allocate_fair_budget(&pending_counts, effective_budget);

        // 5. Per-corpus limit = max(2 * per_corpus_budget, 50) — KTD-2 oversupply.
        // 6. Fetch entities per corpus.
        let mut buckets: Vec<Vec<PendingEntity>> = Vec::new();
        for (i, (hash, _)) in corpus_counts.iter().enumerate() {
            let per_corpus_limit = (assigned[i] as usize * 2).max(50) as u32;
            if assigned[i] == 0 && corpus_counts[i].1 == 0 {
                buckets.push(Vec::new());
                continue;
            }
            let entities = self
                .get_pending_entities_for_corpus(hash, per_corpus_limit)
                .await?;
            buckets.push(entities);
        }

        // 7. Round-robin interleave (KTD-3).
        Ok(interleave_round_robin(buckets))
    }

    /// Get pending entities for a single corpus, with LIMIT.
    async fn get_pending_entities_for_corpus(
        &self,
        docs_root_hash: &str,
        limit: u32,
    ) -> Result<Vec<PendingEntity>> {
        let agent_id = self.db.agent_id.clone();
        let hash = docs_root_hash.to_string();
        let limit = limit as i64;

        self.db
            .with_db(move |db| {
                let sql =
                    "SELECT e.id, e.entity_key, e.type, e.name, e.confidence, e.docs_root_hash,
                            (inv.subject_entity_id IS NOT NULL) AS was_invalidated
                     FROM kg_subject_entities e
                     LEFT JOIN kg_resolutions_log r
                         ON r.subject_entity_id = e.id AND r.agent_id = ?1
                     LEFT JOIN kg_invalidated_no_match inv
                         ON inv.subject_entity_id = e.id AND inv.agent_id = ?1
                     WHERE e.docs_root_hash = ?2
                       AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
                       AND (
                         r.id IS NULL
                         OR r.source_extraction_trace_id != (
                             SELECT cs.extraction_trace_id
                             FROM kg_chunk_subjects cs
                             WHERE cs.subject_entity_id = e.id
                             ORDER BY cs.created_at DESC LIMIT 1
                         )
                       )
                     ORDER BY e.id ASC
                     LIMIT ?3";
                let mut stmt = db.conn.prepare(sql)?;
                let entities: Vec<PendingEntity> = stmt
                    .query_map(rusqlite::params![agent_id, hash, limit], |row| {
                        Ok(PendingEntity {
                            id: row.get(0)?,
                            entity_key: row.get(1)?,
                            entity_type: row.get(2)?,
                            name: row.get(3)?,
                            confidence: row.get(4)?,
                            docs_root_hash: row.get(5)?,
                            was_invalidated: row.get::<_, i64>(6)? != 0,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(entities)
            })
            .await
    }

    /// Count pending entities for a single corpus (lightweight, no row materialisation).
    async fn count_pending_for_corpus(&self, docs_root_hash: &str) -> Result<u32> {
        let agent_id = self.db.agent_id.clone();
        let hash = docs_root_hash.to_string();

        self.db
            .with_db(move |db| {
                let sql = "SELECT COUNT(*)
                     FROM kg_subject_entities e
                     LEFT JOIN kg_resolutions_log r
                         ON r.subject_entity_id = e.id AND r.agent_id = ?1
                     WHERE e.docs_root_hash = ?2
                       AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
                       AND (
                         r.id IS NULL
                         OR r.source_extraction_trace_id != (
                             SELECT cs.extraction_trace_id
                             FROM kg_chunk_subjects cs
                             WHERE cs.subject_entity_id = e.id
                             ORDER BY cs.created_at DESC LIMIT 1
                         )
                       )";
                let mut stmt = db.conn.prepare(sql)?;
                let count: u32 =
                    stmt.query_row(rusqlite::params![agent_id, hash], |row| row.get(0))?;
                Ok(count)
            })
            .await
    }

    /// Get domain candidates of a given type.
    ///
    /// Uses range comparison instead of LIKE to avoid underscore wildcard issues
    /// (e.g., `problem_type:%` would match `problemXtype:foo` with LIKE).
    async fn get_domain_candidates(&self, entity_type: &str) -> Result<Vec<DomainCandidate>> {
        let range_start = format!("{entity_type}:");
        let range_end = format!("{entity_type};"); // ';' is one codepoint above ':' in ASCII

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT id, entity_key, properties_json FROM kg_entities
                     WHERE entity_key >= ?1 AND entity_key < ?2
                     ORDER BY entity_key ASC
                     LIMIT ?3",
                )?;
                let candidates: Vec<DomainCandidate> = stmt
                    .query_map(
                        rusqlite::params![
                            range_start,
                            range_end,
                            MAX_DISAMBIGUATION_CANDIDATES as i64
                        ],
                        |row| {
                            Ok(DomainCandidate {
                                id: row.get(0)?,
                                entity_key: row.get(1)?,
                                properties_json: row.get(2)?,
                            })
                        },
                    )?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(candidates)
            })
            .await
    }

    /// Defensive DB lookup for an LLM-returned `matched_key` not in the in-prompt
    /// candidates slice. Type-bounded via range scan to refuse cross-type matches
    /// at the SQL level (mirrors `get_domain_candidates`). See #874.
    async fn try_domain_entity_by_key(
        &self,
        entity_type: &str,
        matched_key: &str,
    ) -> Result<Option<DomainCandidate>> {
        let range_start = format!("{entity_type}:");
        let range_end = format!("{entity_type};");
        let key = matched_key.to_string();

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT id, entity_key, properties_json FROM kg_entities
                     WHERE entity_key >= ?1 AND entity_key < ?2
                       AND LOWER(entity_key) = LOWER(?3)
                     LIMIT 1",
                )?;
                let result =
                    stmt.query_row(rusqlite::params![range_start, range_end, key], |row| {
                        Ok(DomainCandidate {
                            id: row.get(0)?,
                            entity_key: row.get(1)?,
                            properties_json: row.get(2)?,
                        })
                    });
                match result {
                    Ok(c) => Ok(Some(c)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(anyhow::Error::from(e)),
                }
            })
            .await
    }

    /// Diagnostic-only: detect whether `matched_key` exists in `kg_entities` under
    /// a DIFFERENT type. Used to emit the `cross_type_rejected` event. Returns the
    /// entity_key if found (from which the caller extracts the type prefix).
    async fn try_domain_entity_any_type(&self, matched_key: &str) -> Result<Option<String>> {
        let key = matched_key.to_string();

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT entity_key FROM kg_entities
                     WHERE LOWER(entity_key) = LOWER(?1)
                     LIMIT 1",
                )?;
                let result = stmt.query_row(rusqlite::params![key], |row| row.get::<_, String>(0));
                match result {
                    Ok(ek) => Ok(Some(ek)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(anyhow::Error::from(e)),
                }
            })
            .await
    }

    /// Get source chunk prose for a subject entity (for disambiguation context).
    async fn get_chunk_context(&self, subject_entity_id: i64) -> Result<String> {
        let docs_root_hashes = self.docs_root_hashes.clone();
        let agent_id = self.db.agent_id.clone();

        self.db
            .with_db(move |db| {
                if docs_root_hashes.is_empty() {
                    return Ok(String::new());
                }
                let hash_placeholders: Vec<String> =
                    docs_root_hashes.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT sc.content
                     FROM kg_chunk_subjects cs
                     JOIN search_content sc ON sc.source_type = 'kg_chunk'
                         AND sc.source_id = cs.chunk_id AND sc.agent_id = ?
                     WHERE cs.docs_root_hash IN ({}) AND cs.subject_entity_id = ?
                     LIMIT 3",
                    hash_placeholders.join(","),
                );
                let mut stmt = db.conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id)];
                for h in &docs_root_hashes {
                    params.push(Box::new(h.clone()));
                }
                params.push(Box::new(subject_entity_id));
                let chunks: Vec<String> = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(chunks.join("\n\n---\n\n"))
            })
            .await
    }

    /// Get the latest extraction trace IDs for a set of entity IDs.
    ///
    /// Uses a single batched query with dynamic IN clause (same pattern as
    /// `get_entities_by_ids`) instead of N individual queries.
    async fn get_extraction_trace_ids(
        &self,
        entity_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, String>> {
        let ids = entity_ids.to_vec();
        let docs_root_hashes = self.docs_root_hashes.clone();

        self.db
            .with_db(move |db| {
                if ids.is_empty() || docs_root_hashes.is_empty() {
                    return Ok(std::collections::HashMap::new());
                }

                let hash_placeholders: Vec<String> =
                    docs_root_hashes.iter().map(|_| "?".to_string()).collect();
                let id_placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT subject_entity_id, extraction_trace_id
                     FROM kg_chunk_subjects
                     WHERE docs_root_hash IN ({}) AND subject_entity_id IN ({})
                     ORDER BY created_at DESC",
                    hash_placeholders.join(","),
                    id_placeholders.join(","),
                );
                let mut stmt = db.conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                for h in &docs_root_hashes {
                    params.push(Box::new(h.clone()));
                }
                for id in &ids {
                    params.push(Box::new(*id));
                }

                let mut map = std::collections::HashMap::new();
                let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows.flatten() {
                    // First row per entity_id wins (ORDER BY created_at DESC).
                    map.entry(row.0).or_insert(row.1);
                }
                Ok(map)
            })
            .await
    }

    /// Write a `kg_subject_resolutions` row.
    async fn write_resolution(
        &self,
        subject_entity_id: i64,
        domain_entity_id: i64,
        confidence: f64,
    ) {
        let agent_id = self.db.agent_id.clone();
        let trace_id = self.trace_id.clone();

        if let Err(e) = self
            .db
            .with_db(move |db| {
                db.conn.execute(
                    "INSERT INTO kg_subject_resolutions
                        (agent_id, subject_entity_id, domain_entity_id, confidence, trace_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(agent_id, subject_entity_id, domain_entity_id) DO UPDATE SET
                        confidence = excluded.confidence,
                        trace_id = excluded.trace_id",
                    rusqlite::params![
                        agent_id,
                        subject_entity_id,
                        domain_entity_id,
                        confidence,
                        trace_id,
                    ],
                )?;
                Ok(())
            })
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                subject_entity_id = subject_entity_id,
                error = %e,
                event = "resolution_write_failed",
            );
        }
    }

    /// Write a `kg_resolutions_log` row (UPSERT — one log row per subject entity).
    async fn write_log(
        &self,
        subject_entity_id: i64,
        outcome_val: &str,
        extraction_trace_id: &str,
        model: Option<&str>,
        duration_ms: Option<i64>,
    ) {
        let agent_id = self.db.agent_id.clone();
        let trace_id = self.trace_id.clone();
        let outcome_val = outcome_val.to_owned();
        let extraction_trace_id = extraction_trace_id.to_owned();
        let model = model.map(|s| s.to_owned());

        if let Err(e) = self
            .db
            .with_db(move |db| {
                db.conn.execute(
                    "INSERT INTO kg_resolutions_log
                        (agent_id, subject_entity_id, outcome, resolution_trace_id,
                         source_extraction_trace_id, model, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(agent_id, subject_entity_id) DO UPDATE SET
                        outcome = excluded.outcome,
                        resolution_trace_id = excluded.resolution_trace_id,
                        source_extraction_trace_id = excluded.source_extraction_trace_id,
                        model = excluded.model,
                        duration_ms = excluded.duration_ms,
                        resolved_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                    rusqlite::params![
                        agent_id,
                        subject_entity_id,
                        outcome_val,
                        trace_id,
                        extraction_trace_id,
                        model,
                        duration_ms,
                    ],
                )?;
                // Clean up invalidation marker if present (#961). Defensive
                // DELETE from all code paths (startup, compound-hook, tick).
                crate::db::kg_schema::clear_invalidated_no_match(
                    &db.conn,
                    &agent_id,
                    subject_entity_id,
                )?;
                Ok(())
            })
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                subject_entity_id = subject_entity_id,
                error = %e,
                event = "resolution_log_write_failed",
            );
        }
    }

    // -----------------------------------------------------------------------
    // LLM helpers
    // -----------------------------------------------------------------------

    /// Call LLM with C2.2 retry taxonomy.
    async fn call_llm_with_retry(
        &self,
        llm: &Arc<dyn LlmProvider>,
        request: &LlmRequest,
    ) -> Result<Option<(String, LlmUsage)>> {
        // Attempt 1
        match llm.send_message(request).await {
            Ok(response) => {
                let usage = response.usage.clone();
                Ok(Some((text_content(&response), usage)))
            }
            Err(e) if e.is_retryable() => {
                // Transport/rate-limit retry (up to 2 more attempts)
                for attempt in 1..=2 {
                    let backoff = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(backoff).await;
                    match llm.send_message(request).await {
                        Ok(response) => {
                            let usage = response.usage.clone();
                            return Ok(Some((text_content(&response), usage)));
                        }
                        Err(retry_err) if retry_err.is_retryable() => continue,
                        Err(retry_err) => {
                            warn!(
                                trace_id = %self.trace_id,
                                error = %retry_err,
                                attempt = attempt + 1,
                                event = "resolution_transport_failed",
                            );
                            return Ok(None);
                        }
                    }
                }
                warn!(
                    trace_id = %self.trace_id,
                    event = "resolution_transport_exhausted",
                    "all transport retries exhausted — log-and-skip"
                );
                Ok(None)
            }
            Err(e) => {
                // Configuration error — do not retry (C2.2)
                warn!(
                    trace_id = %self.trace_id,
                    error = %e,
                    event = "resolution_config_error",
                    "non-retryable LLM error — log-and-skip"
                );
                Ok(None)
            }
        }
    }

    /// Retry disambiguation with prompt reinforcement after semantic failure (C2.2).
    ///
    /// Returns the parsed response alongside its usage for separate `llm_calls` row.
    async fn retry_disambiguation_with_reinforcement(
        &self,
        llm: &Arc<dyn LlmProvider>,
        original_request: &LlmRequest,
        bad_output: &str,
    ) -> Result<Option<(DisambiguationResponse, LlmUsage)>> {
        let reinforcement = format!(
            "Your previous response was not valid JSON. The output was:\n{}\n\n\
             Please return ONLY a valid JSON object: {{\"match\": \"<entity_key>\" | null, \"confidence\": 0.0-1.0}}\n\
             No markdown fencing, no explanation, no text outside the JSON.",
            mika_common::text::safe_truncate(bad_output, 500)
        );

        let mut retry_request = original_request.clone();
        retry_request.messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: mika_common::llm::LlmContent::Text(bad_output.to_string()),
        });
        retry_request.messages.push(LlmMessage {
            role: LlmRole::User,
            content: mika_common::llm::LlmContent::Text(reinforcement),
        });

        match llm.send_message(&retry_request).await {
            Ok(response) => {
                let usage = response.usage.clone();
                let text = text_content(&response);
                match parse_disambiguation_json(&text) {
                    Ok(parsed) => Ok(Some((parsed, usage))),
                    Err(e) => {
                        warn!(
                            trace_id = %self.trace_id,
                            error = %e,
                            event = "resolution_semantic_exhausted",
                            "semantic retry also failed — log-and-skip per C2.3"
                        );
                        Ok(None)
                    }
                }
            }
            Err(e) => {
                warn!(
                    trace_id = %self.trace_id,
                    error = %e,
                    event = "resolution_retry_transport_failed",
                );
                Ok(None)
            }
        }
    }

    /// Store an llm_calls row (C2.4).
    async fn store_llm_call(
        &self,
        _entity_key: &str,
        latency_ms: u64,
        provider: &str,
        model: &str,
        usage: &LlmUsage,
        kg_phase: &str,
    ) {
        let call_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        if let Err(e) = self
            .db
            .save_llm_call(
                &call_id,
                RESOLUTION_SESSION_ID,
                Some(&self.trace_id),
                provider,
                model,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
                latency_ms,
                Some("end_turn"),
                "success",
                None,
                0,
                Some(kg_phase),
                None,
                None,
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                error = %e,
                event = "resolution_llm_call_record_failed",
            );
        }
    }

    /// Emit a per-batch audit event (C3.3).
    async fn emit_audit_event(&self, stats: &ResolutionStats) {
        let target_key = format!("kg_resolution:{}", self.trace_id);
        let after_value = format!(
            r#"{{"total":{},"matched_exact":{},"matched_llm":{},"matched_llm_db_fallback":{},"no_match":{},"no_candidate_of_type":{},"skipped_discovered":{},"skipped_no_llm":{},"errors":{}}}"#,
            stats.total,
            stats.matched_exact,
            stats.matched_llm,
            stats.matched_llm_db_fallback,
            stats.no_match,
            stats.no_candidate_of_type,
            stats.skipped_discovered,
            stats.skipped_no_llm,
            stats.errors,
        );

        if let Err(e) = self
            .db
            .log_audit_event(
                RESOLUTION_SESSION_ID,
                "resolve_subject_entity",
                &target_key,
                None,
                Some(&after_value),
                Some("subject entity resolution"),
                Some(&self.trace_id),
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                error = %e,
                event = "resolution_audit_event_failed",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Round-robin interleave (#927 KTD-3)
// ---------------------------------------------------------------------------

/// Interleave entities from multiple corpus buckets in round-robin order.
///
/// Produces `[A_0, B_0, C_0, A_1, B_1, C_1, ...]` — ensures fairness when
/// entities are later processed sequentially with a budget cap.
fn interleave_round_robin(buckets: Vec<Vec<PendingEntity>>) -> Vec<PendingEntity> {
    let total: usize = buckets.iter().map(|b| b.len()).sum();
    let mut result = Vec::with_capacity(total);
    let mut iters: Vec<std::vec::IntoIter<PendingEntity>> =
        buckets.into_iter().map(|b| b.into_iter()).collect();
    loop {
        let mut advanced = false;
        for iter in &mut iters {
            if let Some(entity) = iter.next() {
                result.push(entity);
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------

/// Parse LLM disambiguation response JSON (standalone for testability).
fn parse_disambiguation_json(text: &str) -> Result<DisambiguationResponse> {
    // Strip markdown code fences if present.
    let cleaned = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    serde_json::from_str(cleaned).with_context(|| {
        format!(
            "failed to parse disambiguation JSON: {}",
            mika_common::text::safe_truncate(cleaned, 200)
        )
    })
}

// ---------------------------------------------------------------------------
// Prompt construction (D3)
// ---------------------------------------------------------------------------

/// Build the disambiguation prompt.
fn build_disambiguation_prompt(
    entity: &PendingEntity,
    candidates: &[DomainCandidate],
    chunk_context: &str,
) -> (String, String) {
    let system = r#"You are resolving an entity mention to a canonical knowledge graph node.
Given the entity extracted from prose and a list of candidate domain entities,
determine which candidate (if any) matches. Return null if NONE match —
do not force a pick.

Return ONLY valid JSON matching this schema:
{"match": "<entity_key>" | null, "confidence": 0.0-1.0}

Rules:
- "match" must be one of the candidate entity_key values, or null
- "confidence" is your confidence that the match is correct (0.0 to 1.0)
- If no candidate is a good match, return {"match": null, "confidence": 0.0}
- Consider both the entity name and the source prose context when deciding
- Return ONLY the JSON object, no markdown fencing, no explanation"#;

    let mut user = format!(
        "Extracted entity: {} (confidence: {:.2})\n",
        entity.entity_key, entity.confidence
    );

    if !chunk_context.is_empty() {
        user.push_str(&format!(
            "\nSource prose:\n{}\n",
            mika_common::text::safe_truncate(chunk_context, 2000)
        ));
    }

    user.push_str("\nCandidates:\n");
    for candidate in candidates {
        let desc = candidate
            .properties_json
            .as_deref()
            .and_then(|json| {
                serde_json::from_str::<serde_json::Value>(json)
                    .ok()
                    .and_then(|v| {
                        v.get("description")
                            .and_then(|d| d.as_str().map(String::from))
                    })
            })
            .unwrap_or_default();

        if desc.is_empty() {
            user.push_str(&format!("- {}\n", candidate.entity_key));
        } else {
            user.push_str(&format!("- {} — {}\n", candidate.entity_key, desc));
        }
    }

    (system.to_string(), user)
}

/// Extract text content from LlmResponse.
fn text_content(response: &mika_common::llm::LlmResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|c| match c {
            mika_common::llm::LlmResponseContent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_types_are_not_domain_types() {
        // Discovered types should NOT be in KG_DOMAIN_ENTITY_TYPES.
        assert!(!KG_DOMAIN_ENTITY_TYPES.contains(&"solution_path"));
        assert!(!KG_DOMAIN_ENTITY_TYPES.contains(&"failure_mode"));
        assert!(!KG_DOMAIN_ENTITY_TYPES.contains(&"pattern"));
    }

    #[test]
    fn domain_types_are_resolvable() {
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"skill"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"tool"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"agent"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"problem_type"));
        assert!(KG_DOMAIN_ENTITY_TYPES.contains(&"concept"));
    }

    #[test]
    fn parse_valid_disambiguation_response() {
        let json = r#"{"match": "skill:self-dev", "confidence": 0.85}"#;
        let result = parse_disambiguation_json(json);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.matched, Some("skill:self-dev".to_string()));
        assert!((parsed.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_no_match_response() {
        let json = r#"{"match": null, "confidence": 0.0}"#;
        let result = parse_disambiguation_json(json);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.matched.is_none());
        assert!((parsed.confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_markdown_fenced_response() {
        let json = "```json\n{\"match\": \"tool:run_gh\", \"confidence\": 0.92}\n```";
        let result = parse_disambiguation_json(json);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.matched, Some("tool:run_gh".to_string()));
        assert!((parsed.confidence - 0.92).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_malformed_json_fails() {
        let json = "not valid json";
        let result = parse_disambiguation_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn build_prompt_includes_entity_and_candidates() {
        let entity = PendingEntity {
            id: 1,
            entity_key: "skill:self_dev".to_string(),
            entity_type: "skill".to_string(),
            name: "self_dev".to_string(),
            confidence: 0.85,
            docs_root_hash: "0000000000000000".to_string(),
            was_invalidated: false,
        };

        let candidates = vec![
            DomainCandidate {
                id: 10,
                entity_key: "skill:self-dev".to_string(),
                properties_json: Some(
                    r#"{"description":"Main self-development orchestration"}"#.to_string(),
                ),
            },
            DomainCandidate {
                id: 11,
                entity_key: "skill:self-knowledge".to_string(),
                properties_json: None,
            },
        ];

        let (system, user) = build_disambiguation_prompt(
            &entity,
            &candidates,
            "the self-dev skill handles autonomous implementation",
        );

        assert!(system.contains("Return null if NONE match"));
        assert!(user.contains("skill:self_dev"));
        assert!(user.contains("skill:self-dev"));
        assert!(user.contains("Main self-development orchestration"));
        assert!(user.contains("skill:self-knowledge"));
        assert!(user.contains("self-dev skill handles autonomous implementation"));
    }

    #[test]
    fn confidence_threshold_constant() {
        assert!((EXACT_MATCH_CONFIDENCE_THRESHOLD - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn max_candidates_constant() {
        assert_eq!(MAX_DISAMBIGUATION_CANDIDATES, 50);
    }

    // -----------------------------------------------------------------------
    // F3 path tests (#874) — DB-fallback validation branch
    //
    // These tests exercise the four outcome paths in `disambiguate_with_llm`
    // when the LLM returns a `matched_key`:
    //   Path 1: matched_key in the in-prompt candidate list → matched_llm
    //   Path 2: matched_key not in candidates, exists in kg_entities same type → matched_llm_db_fallback
    //   Path 3: matched_key not in candidates, exists in kg_entities different type → no_match
    //   Path 4: matched_key not in candidates or kg_entities at all → no_match
    //
    // All tests use a subject entity with confidence = 0.8 (below the 0.9
    // exact-match threshold) and a name that does NOT match any domain entity
    // (avoids Stage 1 short-circuit). The mock LLM returns a single JSON
    // response per the disambiguation protocol.
    // -----------------------------------------------------------------------

    /// Helper: create an in-memory AsyncDatabase with a test agent, session,
    /// and corpus mapping — mirrors kg_fixtures::test_db() but avoids
    /// cross-crate test dependency for unit tests in the same module.
    fn f3_test_db() -> crate::async_db::AsyncDatabase {
        let db = crate::db::Database::open_in_memory().unwrap();
        let agent_id = "test-agent";
        db.register_agent(agent_id, agent_id, "/tmp/mika-test/test-agent")
            .unwrap();
        db.register_agent_corpus(agent_id, "0000000000000000", "/test/docs/solutions")
            .unwrap();
        db.create_session("test-session", agent_id, "test").unwrap();
        crate::async_db::AsyncDatabase::new_with_agent(db, agent_id)
    }

    /// Helper: seed a domain entity into kg_entities.
    async fn f3_seed_domain_entity(
        db: &crate::async_db::AsyncDatabase,
        entity_type: &str,
        name: &str,
    ) -> i64 {
        let entity_key = format!("{entity_type}:{name}");
        let etype = entity_type.to_string();
        let name = name.to_string();
        db.with_db(move |db| {
            db.execute_sql(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES (?1, ?2, ?3)",
                &[&entity_key as &dyn rusqlite::types::ToSql, &etype, &name],
            )?;
            Ok(db.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    /// Helper: seed a subject entity into kg_subject_entities.
    async fn f3_seed_subject_entity(
        db: &crate::async_db::AsyncDatabase,
        entity_type: &str,
        name: &str,
        confidence: f64,
    ) -> i64 {
        let entity_key = format!("{entity_type}:{name}");
        let etype = entity_type.to_string();
        let name = name.to_string();
        db.with_db(move |db| {
            db.execute_sql(
                "INSERT INTO kg_subject_entities \
                 (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES ('0000000000000000', '/test/docs/solutions', ?1, ?2, ?3, ?4)",
                &[
                    &entity_key as &dyn rusqlite::types::ToSql,
                    &etype,
                    &name,
                    &confidence,
                ],
            )?;
            Ok(db.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    /// Helper: read resolution log outcome for a subject entity.
    async fn f3_get_outcome(
        db: &crate::async_db::AsyncDatabase,
        subject_entity_id: i64,
    ) -> Option<String> {
        let agent_id = db.agent_id.clone();
        db.with_db(move |db| {
            let result = db.conn.query_row(
                "SELECT outcome FROM kg_resolutions_log \
                 WHERE agent_id = ?1 AND subject_entity_id = ?2",
                rusqlite::params![agent_id, subject_entity_id],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(v) => Ok(Some(v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow::Error::from(e)),
            }
        })
        .await
        .unwrap()
    }

    /// Helper: check if a resolution row exists.
    async fn f3_get_resolution(
        db: &crate::async_db::AsyncDatabase,
        subject_entity_id: i64,
    ) -> Option<(i64, f64)> {
        let agent_id = db.agent_id.clone();
        db.with_db(move |db| {
            let result = db.conn.query_row(
                "SELECT domain_entity_id, confidence FROM kg_subject_resolutions \
                 WHERE agent_id = ?1 AND subject_entity_id = ?2",
                rusqlite::params![agent_id, subject_entity_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            );
            match result {
                Ok(v) => Ok(Some(v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow::Error::from(e)),
            }
        })
        .await
        .unwrap()
    }

    /// F3 Path 1: LLM candidate is in the resolved (in-prompt) list.
    /// The matched_key appears in `get_domain_candidates` output (≤50 entries).
    /// Expected: outcome = matched_llm, resolution row written.
    #[tokio::test]
    async fn f3_path1_in_list_accept() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed a domain entity that WILL be in the candidate list (< 50 skills).
        let domain_id = f3_seed_domain_entity(&db, "skill", "self-dev").await;

        // Seed a subject entity with a DIFFERENT name so Stage 1 exact match misses.
        // Confidence = 0.8 < 0.9 threshold → falls through to Stage 2.
        let subject_id = f3_seed_subject_entity(&db, "skill", "selfdev_typo", 0.8).await;

        // Mock LLM returns the domain entity key (which IS in the candidate list).
        let mock = MockLlmProvider::builder()
            .response(text_response(
                r#"{"match": "skill:self-dev", "confidence": 0.85}"#,
            ))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-f3-path1"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-1", 100)
            .await
            .unwrap();

        assert_eq!(stats.matched_llm, 1, "Path 1 should produce matched_llm");
        assert_eq!(
            stats.matched_llm_db_fallback, 0,
            "Path 1 should NOT trigger DB fallback"
        );

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::MATCHED_LLM));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(resolution.is_some(), "resolution row should be written");
        let (resolved_domain_id, _) = resolution.unwrap();
        assert_eq!(resolved_domain_id, domain_id);
    }

    /// F3 Path 2: DB-fallback hit, same type.
    /// The LLM returns a matched_key that exists in kg_entities under the same
    /// type but is NOT in the 50-cap in-prompt candidate list.
    /// Expected: outcome = matched_llm_db_fallback, counter incremented.
    #[tokio::test]
    async fn f3_path2_db_fallback_same_type() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed 51 domain skill entities so "skill:zzz-target" (alphabetically last)
        // falls outside the 50-cap candidate window.
        for i in 0..50 {
            f3_seed_domain_entity(&db, "skill", &format!("aaa-filler-{i:03}")).await;
        }
        // This one sorts after all "aaa-filler-*" entries → outside the 50-cap.
        let target_id = f3_seed_domain_entity(&db, "skill", "zzz-target").await;

        // Subject entity with a name that won't exact-match anything.
        let subject_id = f3_seed_subject_entity(&db, "skill", "zzz_target_typo", 0.8).await;

        // Mock LLM returns "skill:zzz-target" — not in the in-prompt list but
        // exists in kg_entities under the same type.
        let mock = MockLlmProvider::builder()
            .response(text_response(
                r#"{"match": "skill:zzz-target", "confidence": 0.90}"#,
            ))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-f3-path2"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-2", 100)
            .await
            .unwrap();

        assert_eq!(
            stats.matched_llm_db_fallback, 1,
            "Path 2 should increment matched_llm_db_fallback"
        );
        assert_eq!(
            stats.matched_llm, 0,
            "Path 2 should NOT count as regular matched_llm"
        );

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::MATCHED_LLM_DB_FALLBACK));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(resolution.is_some(), "resolution row should be written");
        let (resolved_domain_id, _) = resolution.unwrap();
        assert_eq!(resolved_domain_id, target_id);
    }

    /// F3 Path 3: Cross-type rejected.
    /// The LLM returns a matched_key that exists in kg_entities but under a
    /// DIFFERENT entity type than the subject. Type-bounded DB lookup rejects it.
    /// Expected: outcome = no_match, no resolution row.
    #[tokio::test]
    async fn f3_path3_cross_type_rejected() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed a domain entity under "tool" type.
        f3_seed_domain_entity(&db, "tool", "run_gh").await;

        // Also seed at least one "skill" domain entity so the candidate list
        // is non-empty (empty candidates → early return before LLM call).
        f3_seed_domain_entity(&db, "skill", "some-skill").await;

        // Subject entity is type "skill".
        let subject_id = f3_seed_subject_entity(&db, "skill", "run_gh_typo", 0.8).await;

        // Mock LLM returns "tool:run_gh" — exists in kg_entities but wrong type.
        let mock = MockLlmProvider::builder()
            .response(text_response(
                r#"{"match": "tool:run_gh", "confidence": 0.80}"#,
            ))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-f3-path3"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-3", 100)
            .await
            .unwrap();

        // #1154: Stage 1 returned Ok(None) (subject entity_key doesn't match
        // any domain entity), so even though the LLM tried a cross-type match,
        // the outcome is no_candidate_of_type (the subject is a phantom).
        assert_eq!(
            stats.no_candidate_of_type, 1,
            "Path 3 should produce no_candidate_of_type (Stage 1 was Ok(None))"
        );
        assert_eq!(stats.no_match, 0);
        assert_eq!(stats.matched_llm, 0);
        assert_eq!(stats.matched_llm_db_fallback, 0);

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::NO_CANDIDATE_OF_TYPE));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(
            resolution.is_none(),
            "no resolution row should be written for cross-type rejection"
        );
    }

    /// F3 Path 4: DB miss — matched_key does not exist in kg_entities at all.
    /// Expected: outcome = no_candidate_of_type (Stage 1 returned Ok(None)),
    /// no resolution row, WARN log with db_fallback_attempted=true and
    /// db_fallback_hit=false fields.
    #[tokio::test]
    async fn f3_path4_db_miss() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed one domain skill entity so the candidate list is non-empty.
        f3_seed_domain_entity(&db, "skill", "some-real-skill").await;

        // Subject entity.
        let subject_id = f3_seed_subject_entity(&db, "skill", "nonexistent_typo", 0.8).await;

        // Mock LLM returns a key that does NOT exist in kg_entities at all.
        let mock = MockLlmProvider::builder()
            .response(text_response(
                r#"{"match": "skill:totally-made-up", "confidence": 0.75}"#,
            ))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-f3-path4"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-4", 100)
            .await
            .unwrap();

        // #1154: Stage 1 returned Ok(None) (subject entity_key doesn't match
        // any domain entity), so the outcome is no_candidate_of_type.
        assert_eq!(
            stats.no_candidate_of_type, 1,
            "Path 4 should produce no_candidate_of_type (Stage 1 was Ok(None))"
        );
        assert_eq!(stats.no_match, 0);
        assert_eq!(stats.matched_llm, 0);
        assert_eq!(stats.matched_llm_db_fallback, 0);

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::NO_CANDIDATE_OF_TYPE));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(
            resolution.is_none(),
            "no resolution row should be written for DB miss"
        );
    }

    // -----------------------------------------------------------------------
    // #961: reattempted_no_match counter tests
    // -----------------------------------------------------------------------

    /// Helper: seed invalidation markers into `kg_invalidated_no_match`.
    async fn seed_invalidation_markers(
        db: &crate::async_db::AsyncDatabase,
        subject_entity_ids: &[i64],
    ) {
        let ids = subject_entity_ids.to_vec();
        let agent_id = db.agent_id.clone();
        db.with_db(move |db| {
            crate::db::kg_schema::record_invalidated_no_match(&db.conn, &agent_id, &ids)?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Helper: count rows in `kg_invalidated_no_match` for this agent.
    async fn count_invalidation_markers(db: &crate::async_db::AsyncDatabase) -> usize {
        let agent_id = db.agent_id.clone();
        db.with_db(move |db| {
            let count: i64 = db.conn.query_row(
                "SELECT COUNT(*) FROM kg_invalidated_no_match WHERE agent_id = ?1",
                rusqlite::params![agent_id],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
        .await
        .unwrap()
    }

    /// Non-zero path: entities with invalidation markers are counted in
    /// `reattempted_no_match`. Markers are cleaned up after resolution.
    #[tokio::test]
    async fn reattempted_no_match_nonzero_with_markers() {
        let db = f3_test_db();

        // Seed a domain entity for exact match.
        f3_seed_domain_entity(&db, "concept", "cross-repo:worktree-isolation").await;

        // Seed 3 concept-typed subject entities (no log rows → pending via
        // r.id IS NULL). Two match the domain entity, one does not.
        let s1 =
            f3_seed_subject_entity(&db, "concept", "cross-repo:worktree-isolation", 0.95).await;
        let s2 = f3_seed_subject_entity(&db, "concept", "unmatched-concept-a", 0.95).await;
        let s3 = f3_seed_subject_entity(&db, "concept", "unmatched-concept-b", 0.95).await;

        // Mark all 3 as invalidated (simulates #960 rebuild).
        seed_invalidation_markers(&db, &[s1, s2, s3]).await;
        assert_eq!(count_invalidation_markers(&db).await, 3);

        // Resolve without LLM (exact-match only). s1 matches, s2/s3 get
        // skipped_no_llm (no LLM configured).
        let resolver = SubjectEntityResolver::new(
            db.clone(),
            None, // no LLM → exact-match only
            vec!["0000000000000000".to_string()],
            Some("test-reattempt-nonzero"),
        );

        let stats = resolver.resolve_pending(100).await.unwrap();

        // All 3 were invalidated and resolved (wrote log rows).
        assert_eq!(
            stats.reattempted_no_match, 3,
            "all 3 invalidated entities should be counted"
        );
        assert_eq!(stats.matched_exact, 1, "s1 should exact-match");
        assert_eq!(stats.skipped_no_llm, 2, "s2/s3 have no LLM → skipped");

        // Markers should be cleaned up.
        assert_eq!(
            count_invalidation_markers(&db).await,
            0,
            "all markers should be cleaned up after resolution"
        );
    }

    /// Zero path: entities without invalidation markers produce
    /// `reattempted_no_match=0`.
    #[tokio::test]
    async fn reattempted_no_match_zero_without_markers() {
        let db = f3_test_db();

        // Seed a domain entity and subject entities but NO markers.
        f3_seed_domain_entity(&db, "concept", "infra:helm-values").await;
        f3_seed_subject_entity(&db, "concept", "infra:helm-values", 0.95).await;
        f3_seed_subject_entity(&db, "concept", "some-other-concept", 0.95).await;

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            None,
            vec!["0000000000000000".to_string()],
            Some("test-reattempt-zero"),
        );

        let stats = resolver.resolve_pending(100).await.unwrap();

        assert_eq!(
            stats.reattempted_no_match, 0,
            "no markers → reattempted_no_match should be 0"
        );
        assert!(stats.total > 0, "should have resolved some entities");
    }

    /// Mixed path: only invalidated entities increment the counter.
    #[tokio::test]
    async fn reattempted_no_match_mixed_invalidated_and_fresh() {
        let db = f3_test_db();

        f3_seed_domain_entity(&db, "concept", "cross-repo:branch-naming").await;

        // s1: invalidated (marker present).
        let s1 = f3_seed_subject_entity(&db, "concept", "cross-repo:branch-naming", 0.95).await;
        // s2: fresh (no marker).
        let _s2 = f3_seed_subject_entity(&db, "concept", "fresh-concept", 0.95).await;

        seed_invalidation_markers(&db, &[s1]).await;

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            None,
            vec!["0000000000000000".to_string()],
            Some("test-reattempt-mixed"),
        );

        let stats = resolver.resolve_pending(100).await.unwrap();

        assert_eq!(
            stats.reattempted_no_match, 1,
            "only s1 (invalidated) should be counted"
        );
        assert_eq!(stats.total, 2, "both entities should be processed");
    }

    /// Edge case: invalidated entity that resolves as `no_match` again
    /// (no domain match available) — still counted in `reattempted_no_match`,
    /// marker cleaned up, new `no_match` log row written.
    #[tokio::test]
    async fn reattempted_no_match_resolves_as_no_match_again() {
        let db = f3_test_db();

        // No domain entity for this type → will resolve as skipped_no_llm
        // (no LLM configured) which still writes a log row.
        let s1 = f3_seed_subject_entity(&db, "concept", "orphaned-concept", 0.95).await;

        seed_invalidation_markers(&db, &[s1]).await;

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            None,
            vec!["0000000000000000".to_string()],
            Some("test-reattempt-no-match-again"),
        );

        let stats = resolver.resolve_pending(100).await.unwrap();

        assert_eq!(
            stats.reattempted_no_match, 1,
            "invalidated entity should be counted even if it resolves as skipped_no_llm"
        );
        assert_eq!(
            count_invalidation_markers(&db).await,
            0,
            "marker should be cleaned up"
        );

        // Verify a log row was written.
        let outcome = f3_get_outcome(&db, s1).await;
        assert_eq!(
            outcome.as_deref(),
            Some(outcome::SKIPPED_NO_LLM),
            "should write skipped_no_llm log row"
        );
    }

    // -----------------------------------------------------------------------
    // #1154: no_candidate_of_type outcome split tests
    // -----------------------------------------------------------------------

    /// #1154 AC7a — Phantom subject: Stage 1 returns Ok(None) (no exact match),
    /// Stage 2 returns Ok(None) (LLM says no match) → outcome = no_candidate_of_type.
    /// This is the primary scenario from the experiment: extractor produces
    /// `agent:vincent` but no domain entity with that entity_key exists.
    #[tokio::test]
    async fn no_candidate_of_type_phantom_subject() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed a domain entity under "agent" type so the candidate list is
        // non-empty (prevents the candidates.is_empty() short-circuit from
        // conflating with this test's target path).
        f3_seed_domain_entity(&db, "agent", "mika-dev").await;

        // Subject entity with a name that does NOT match any domain entity.
        // Stage 1 will return Ok(None). Confidence 0.8 < 0.9 threshold is
        // irrelevant here since Stage 1 returns None anyway.
        let subject_id = f3_seed_subject_entity(&db, "agent", "vincent", 0.85).await;

        // Mock LLM says no match — returns null for the match field.
        let mock = MockLlmProvider::builder()
            .response(text_response(r#"{"match": null, "confidence": 0.0}"#))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-1154-phantom"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-1154a", 100)
            .await
            .unwrap();

        assert_eq!(
            stats.no_candidate_of_type, 1,
            "phantom subject should produce no_candidate_of_type"
        );
        assert_eq!(stats.no_match, 0, "should NOT produce no_match");

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::NO_CANDIDATE_OF_TYPE));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(
            resolution.is_none(),
            "no resolution row should be written for phantom subject"
        );
    }

    /// #1154 AC7b — Disambig failure: Stage 1 returns Ok(Some) with low
    /// confidence (domain entity exists but extraction confidence < 0.9),
    /// Stage 2 returns Ok(None) (LLM rejects) → outcome = no_match.
    /// Narrowed semantic: "candidates existed, LLM couldn't disambiguate."
    #[tokio::test]
    async fn no_match_disambig_failure_with_existing_candidate() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed a domain entity that WILL exact-match the subject's entity_key.
        f3_seed_domain_entity(&db, "agent", "mika-dev").await;

        // Subject entity with the SAME name — Stage 1 finds it. But
        // confidence = 0.5 < 0.9 threshold → escalates to Stage 2.
        let subject_id = f3_seed_subject_entity(&db, "agent", "mika-dev", 0.5).await;

        // Mock LLM says no match despite having the exact candidate.
        let mock = MockLlmProvider::builder()
            .response(text_response(r#"{"match": null, "confidence": 0.0}"#))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-1154-disambig-fail"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-1154b", 100)
            .await
            .unwrap();

        assert_eq!(
            stats.no_match, 1,
            "disambig failure with existing candidate should produce no_match"
        );
        assert_eq!(
            stats.no_candidate_of_type, 0,
            "should NOT produce no_candidate_of_type"
        );

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::NO_MATCH));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(
            resolution.is_none(),
            "no resolution row should be written for disambig failure"
        );
    }

    /// #1154 AC7c — Empty type bucket: no domain entities of the subject's type
    /// exist at all → candidates.is_empty() short-circuit → no_candidate_of_type.
    /// Distinct from the phantom test: here there are ZERO domain entities of
    /// the type, so the LLM is never called.
    #[tokio::test]
    async fn no_candidate_of_type_empty_type_bucket() {
        use mika_common::llm::mock::{MockLlmProvider, text_response};

        let db = f3_test_db();

        // Seed NO domain entities of type "problem_type".
        // (Seed one of a different type to ensure the DB is not empty.)
        f3_seed_domain_entity(&db, "skill", "some-skill").await;

        // Subject entity of type "problem_type" — no candidates exist.
        let subject_id =
            f3_seed_subject_entity(&db, "problem_type", "nonexistent_problem", 0.9).await;

        // Mock LLM should NOT be called (candidates list is empty, short-circuits).
        // Provide a response anyway so the test doesn't hang if logic is wrong.
        let mock = MockLlmProvider::builder()
            .response(text_response(
                r#"{"match": "problem_type:fabrication", "confidence": 0.9}"#,
            ))
            .build();

        let resolver = SubjectEntityResolver::new(
            db.clone(),
            Some(Arc::new(mock)),
            vec!["0000000000000000".to_string()],
            Some("test-1154-empty-bucket"),
        );

        let stats = resolver
            .resolve_doc_entities(&[subject_id], "extraction-trace-1154c", 100)
            .await
            .unwrap();

        assert_eq!(
            stats.no_candidate_of_type, 1,
            "empty type bucket should produce no_candidate_of_type"
        );
        assert_eq!(stats.no_match, 0, "should NOT produce no_match");
        assert_eq!(
            stats.llm_calls, 0,
            "LLM should not be called when candidates list is empty"
        );

        let outcome = f3_get_outcome(&db, subject_id).await;
        assert_eq!(outcome.as_deref(), Some(outcome::NO_CANDIDATE_OF_TYPE));

        let resolution = f3_get_resolution(&db, subject_id).await;
        assert!(
            resolution.is_none(),
            "no resolution row should be written for empty type bucket"
        );
    }
}
