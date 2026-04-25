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
//! ## Resolution Pipeline
//!
//! For each subject entity with a well-known type (skill, tool, agent, problem_type):
//!
//! 1. **Exact match:** Case-insensitive match against `kg_entities.entity_key`.
//!    If match found and extraction confidence > 0.9, resolve immediately.
//! 2. **LLM disambiguation:** For unmatched or low-confidence entities, send to
//!    the resolution LLM with candidate list and source chunk context.
//! 3. **Tracking:** Every resolution attempt writes a `kg_resolutions_log` row.
//!
//! Discovered types (solution_path, failure_mode, pattern) skip resolution
//! entirely — no domain counterpart exists (D8).

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use mika_common::llm::{LlmMessage, LlmProvider, LlmRequest, LlmRole};

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
    pub const NO_MATCH: &str = "no_match";
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
    /// LLM picked a match.
    LlmMatch {
        domain_entity_id: i64,
        confidence: f64,
    },
    /// LLM said no match or no candidates found.
    NoMatch,
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
    pub no_match: usize,
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

        let pending = self.get_pending_entities().await?;

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
            no_match = stats.no_match,
            skipped_discovered = stats.skipped_discovered,
            skipped_no_llm = stats.skipped_no_llm,
            errors = stats.errors,
            llm_calls = stats.llm_calls,
            aborted_budget = stats.aborted_budget,
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
        match self.try_exact_match(entity).await {
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
            }
            Ok(None) => {
                // No exact match — escalate to LLM.
            }
            Err(e) => {
                return (
                    ResolutionResult::Error(format!("exact match query failed: {e}")),
                    false,
                );
            }
        }

        // Stage 2: LLM disambiguation (D1 stage 2).
        let Some(ref llm) = self.llm else {
            return (ResolutionResult::SkippedNoLlm, false);
        };

        // #757: respect per-batch budget before issuing the LLM call.
        if !llm_call_allowed {
            return (ResolutionResult::SkippedBudget, false);
        }

        match self.disambiguate_with_llm(llm, entity).await {
            Ok(Some((domain_id, llm_confidence))) => {
                // D2: confidence = min(extraction_confidence, llm_confidence).
                let combined_confidence = entity.confidence.min(llm_confidence);
                (
                    ResolutionResult::LlmMatch {
                        domain_entity_id: domain_id,
                        confidence: combined_confidence,
                    },
                    true,
                )
            }
            Ok(None) => (ResolutionResult::NoMatch, true),
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
    async fn disambiguate_with_llm(
        &self,
        llm: &Arc<dyn LlmProvider>,
        entity: &PendingEntity,
    ) -> Result<Option<(i64, f64)>> {
        let agent_id = self.db.agent_id.clone();

        // 1. Fetch domain candidates of the same type (bounded to MAX_DISAMBIGUATION_CANDIDATES).
        let candidates = self.get_domain_candidates(&entity.entity_type).await?;

        if candidates.is_empty() {
            // No domain entities of this type → no match possible.
            return Ok(None);
        }

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
        let response_text = match self.call_llm_with_retry(llm, &request).await? {
            Some(text) => text,
            None => return Ok(None), // All retries exhausted, log-and-skip.
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        // 5. Parse and validate response. Semantic retry on malformed JSON (C2.2).
        let parsed = match parse_disambiguation_json(&response_text) {
            Ok(p) => p,
            Err(parse_err) => {
                warn!(
                    trace_id = %self.trace_id,
                    entity_key = %entity.entity_key,
                    error = %parse_err,
                    event = "resolution_parse_failed_retry",
                    "malformed JSON from LLM — retrying with reinforcement"
                );
                match self
                    .retry_disambiguation_with_reinforcement(llm, &request, &response_text)
                    .await
                {
                    Ok(Some(p)) => p,
                    Ok(None) | Err(_) => return Ok(None), // Log-and-skip per C2.3.
                }
            }
        };

        // Store llm_calls row (C2.4).
        self.store_llm_call(
            &entity.entity_key,
            latency_ms,
            llm.provider_name(),
            llm.model_name(),
        )
        .await;

        match parsed.matched {
            Some(matched_key) => {
                // Validate: matched entity_key must be in the candidate list.
                if let Some(candidate) = candidates
                    .iter()
                    .find(|c| c.entity_key.eq_ignore_ascii_case(&matched_key))
                {
                    if (0.0..=1.0).contains(&parsed.confidence) {
                        Ok(Some((candidate.id, parsed.confidence)))
                    } else {
                        warn!(
                            trace_id = %self.trace_id,
                            agent_id = %agent_id,
                            entity_key = %entity.entity_key,
                            confidence = parsed.confidence,
                            event = "resolution_confidence_out_of_range",
                        );
                        Ok(None)
                    }
                } else {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        entity_key = %entity.entity_key,
                        matched_key = %matched_key,
                        event = "resolution_matched_key_not_in_candidates",
                        "LLM returned entity_key not in candidate list — treating as no_match"
                    );
                    Ok(None)
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
                    "SELECT id, entity_key, type, name, confidence
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
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(entities)
            })
            .await
    }

    /// Get pending entities — never attempted or stale from re-extraction (D4).
    /// Scoped to all the agent's docs_root_hashes via IN-list (#798).
    async fn get_pending_entities(&self) -> Result<Vec<PendingEntity>> {
        let agent_id = self.db.agent_id.clone();
        let docs_root_hashes = self.docs_root_hashes.clone();

        self.db
            .with_db(move |db| {
                if docs_root_hashes.is_empty() {
                    return Ok(Vec::new());
                }
                let hash_placeholders: Vec<String> =
                    docs_root_hashes.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT e.id, e.entity_key, e.type, e.name, e.confidence
                     FROM kg_subject_entities e
                     LEFT JOIN kg_resolutions_log r
                         ON r.subject_entity_id = e.id AND r.agent_id = ?
                     WHERE e.docs_root_hash IN ({})
                       AND e.type IN ('skill', 'tool', 'agent', 'problem_type')
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
                // agent_id is first param, then hash params
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id)];
                for h in &docs_root_hashes {
                    params.push(Box::new(h.clone()));
                }
                let entities: Vec<PendingEntity> = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        Ok(PendingEntity {
                            id: row.get(0)?,
                            entity_key: row.get(1)?,
                            entity_type: row.get(2)?,
                            name: row.get(3)?,
                            confidence: row.get(4)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(entities)
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
    ) -> Result<Option<String>> {
        // Attempt 1
        match llm.send_message(request).await {
            Ok(response) => Ok(Some(text_content(&response))),
            Err(e) if e.is_retryable() => {
                // Transport/rate-limit retry (up to 2 more attempts)
                for attempt in 1..=2 {
                    let backoff = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(backoff).await;
                    match llm.send_message(request).await {
                        Ok(response) => return Ok(Some(text_content(&response))),
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
    async fn retry_disambiguation_with_reinforcement(
        &self,
        llm: &Arc<dyn LlmProvider>,
        original_request: &LlmRequest,
        bad_output: &str,
    ) -> Result<Option<DisambiguationResponse>> {
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
                let text = text_content(&response);
                match parse_disambiguation_json(&text) {
                    Ok(parsed) => Ok(Some(parsed)),
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
    async fn store_llm_call(&self, entity_key: &str, latency_ms: u64, provider: &str, model: &str) {
        let call_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        if let Err(e) = self
            .db
            .save_llm_call(
                &call_id,
                RESOLUTION_SESSION_ID,
                Some(&self.trace_id),
                provider,
                model,
                0,
                0,
                None,
                None,
                latency_ms,
                Some("end_turn"),
                "success",
                None,
                0,
                Some("kg_resolution"),
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                entity_key = %entity_key,
                error = %e,
                event = "resolution_llm_call_record_failed",
            );
        }
    }

    /// Emit a per-batch audit event (C3.3).
    async fn emit_audit_event(&self, stats: &ResolutionStats) {
        let target_key = format!("kg_resolution:{}", self.trace_id);
        let after_value = format!(
            r#"{{"total":{},"matched_exact":{},"matched_llm":{},"no_match":{},"skipped_discovered":{},"skipped_no_llm":{},"errors":{}}}"#,
            stats.total,
            stats.matched_exact,
            stats.matched_llm,
            stats.no_match,
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
}
