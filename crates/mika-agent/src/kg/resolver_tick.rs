//! Periodic extraction + resolution tick for draining KG backlogs.
//!
//! Decouples extraction and resolution drain rates from restart cadence
//! (#906, #1052). Runs every 30 minutes per KG-enabled agent:
//!
//! 1. **Extraction phase** (#1052): counts pending docs per corpus, allocates
//!    budget fairly via `allocate_fair_budget`, runs `extract_pending` for each
//!    corpus. This ensures corpora that don't fully drain at startup get 48
//!    more extraction opportunities per day.
//!
//! 2. **Resolution phase** (#906): runs `resolve_pending(budget)` to bridge
//!    newly-extracted and previously-pending subject entities to domain graph.
//!
//! Both phases use the same `MIKA_KG_BATCH_BUDGET` (default 500), preserving
//! the "no silent multi-thousand-call bursts" invariant from #757.
//!
//! The tick joins the startup background spawn and compound-hook synchronous
//! spawn as the third execution context for both extraction and resolution.
//! `kg_extractions UNIQUE(docs_root_hash, source_doc_path)` and
//! `kg_resolutions_log UNIQUE(agent_id, subject_entity_id)` serve as
//! deduplication mechanisms. Concurrent writes are serialized by SQLite
//! WAL; both writes are functionally idempotent (last-writer-wins on the
//! hash field for extraction, first-writer-wins for resolution).
//!
//! Pattern follows `server::checkpoint::spawn_dashboard_checkpoint_task()`:
//! interval + fail-open (log-and-skip) + lifecycle tied to tokio runtime drop.

use crate::async_db::AsyncDatabase;
use crate::kg::config::KgAgentConfig;
use crate::kg::entity_resolver::SubjectEntityResolver;
use crate::kg::subject_extractor::SubjectExtractor;
use mika_common::llm::LlmProvider;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Tick interval in seconds. Hard-coded for v1 (#906).
/// Future tunable: `MIKA_KG_RESOLVER_TICK_INTERVAL_SECS`.
const RESOLVER_TICK_INTERVAL_SECS: u64 = 30 * 60;

/// Spawns a background tokio task that runs extraction + resolution every
/// [`RESOLVER_TICK_INTERVAL_SECS`] for a single KG-enabled agent.
///
/// The first immediate fire is skipped so the startup background tasks
/// handle the immediate post-restart drain. Subsequent ticks fire on the
/// 30-min cadence.
///
/// # Arguments
///
/// * `agent_id` — Agent name (for logging and DB scoping).
/// * `db` — Async database handle carrying the agent_id.
/// * `extraction_llm` — Optional extraction LLM provider. `None` = skip
///   extraction phase (resolution-only tick, pre-#1052 behavior).
/// * `resolution_llm` — Optional resolution LLM provider. `None` = exact-match-only.
/// * `kg_config` — Agent's KG configuration. If `Disabled`, the task exits
///   immediately.
/// * `budget` — Per-batch LLM call cap (from `MIKA_KG_BATCH_BUDGET`).
/// * `interval_secs` — Tick interval override (for testing). Pass `None` to
///   use the default 30-minute interval.
pub fn spawn_resolver_tick_task(
    agent_id: String,
    db: AsyncDatabase,
    extraction_llm: Option<Arc<dyn LlmProvider>>,
    resolution_llm: Option<Arc<dyn LlmProvider>>,
    kg_config: &KgAgentConfig,
    budget: u32,
    interval_secs: Option<u64>,
) -> tokio::task::JoinHandle<()> {
    let (docs_root_hashes, corpora_roots): (Vec<String>, Vec<PathBuf>) = match kg_config {
        KgAgentConfig::Enabled { corpora } => (
            corpora.iter().map(|c| c.docs_root_hash.clone()).collect(),
            corpora.iter().map(|c| c.docs_root.clone()).collect(),
        ),
        KgAgentConfig::Disabled { .. } => (Vec::new(), Vec::new()),
    };

    tokio::spawn(async move {
        // Skip if KG disabled for this agent.
        if docs_root_hashes.is_empty() {
            return;
        }

        let secs = interval_secs.unwrap_or(RESOLVER_TICK_INTERVAL_SECS);
        let mut interval = tokio::time::interval(Duration::from_secs(secs));
        // Skip the first immediate fire — startup spawn covers it.
        interval.tick().await;

        loop {
            interval.tick().await;
            tick_body(
                &agent_id,
                &db,
                &extraction_llm,
                &resolution_llm,
                &corpora_roots,
                &docs_root_hashes,
                budget,
            )
            .await;
        }
    })
}

async fn tick_body(
    agent_id: &str,
    db: &AsyncDatabase,
    extraction_llm: &Option<Arc<dyn LlmProvider>>,
    resolution_llm: &Option<Arc<dyn LlmProvider>>,
    corpora_roots: &[PathBuf],
    docs_root_hashes: &[String],
    budget: u32,
) {
    let trace_id = mika_common::trace::generate_trace_id();

    // --- Phase 1: Extraction (#1052) ---
    // Run extraction before resolution so newly-extracted entities are
    // immediately available for resolution in the same tick.
    if let Some(ext_llm) = extraction_llm {
        tick_extraction(agent_id, db, ext_llm, corpora_roots, budget, &trace_id).await;
    }

    // --- Phase 2: Resolution (#906) ---
    tick_resolution(
        agent_id,
        db,
        resolution_llm,
        docs_root_hashes,
        budget,
        &trace_id,
    )
    .await;

    // --- Phase 3: Coverage report (#1052) ---
    // Log per-corpus extraction coverage after both phases complete.
    // Only runs when extraction LLM is configured (otherwise no extractors
    // to query coverage from).
    if let Some(ext_llm) = extraction_llm {
        tick_coverage(agent_id, db, ext_llm, corpora_roots, &trace_id).await;
    }
}

/// Extraction phase of the periodic tick (#1052).
///
/// Counts pending docs per corpus, allocates budget fairly, then runs
/// `extract_pending` for each corpus. Structurally identical to startup
/// extraction in `server/mod.rs` — same `SubjectExtractor::extract_pending()`
/// call, same fair budget allocation via `allocate_fair_budget()`.
async fn tick_extraction(
    agent_id: &str,
    db: &AsyncDatabase,
    llm: &Arc<dyn LlmProvider>,
    corpora_roots: &[PathBuf],
    budget: u32,
    trace_id: &str,
) {
    // Phase 1: Count pending docs per corpus.
    let mut corpus_pending: Vec<u32> = Vec::new();
    let mut extractors: Vec<SubjectExtractor> = Vec::new();
    for docs_root in corpora_roots {
        let extractor =
            SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), Some(trace_id));
        let count = match extractor.count_pending_docs().await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    target: "mika::otel",
                    trace_id = %trace_id,
                    agent_id = %agent_id,
                    docs_root = %docs_root.display(),
                    error = %e,
                    event = "kg_extraction_tick.count_error",
                    "failed to count pending docs for corpus — treating as 0 pending"
                );
                0
            }
        };
        corpus_pending.push(count);
        extractors.push(extractor);
    }

    let total_pending: u32 = corpus_pending.iter().sum();
    if total_pending == 0 {
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            total_pending = 0,
            event = "kg_extraction_tick.complete",
            "no pending docs — extraction tick is a no-op"
        );
        return;
    }

    // Phase 2: Fair budget allocation via shared function.
    let allocated = crate::kg::budget::allocate_fair_budget(&corpus_pending, budget);

    // Phase 3: Execute extractors with per-corpus budgets.
    let mut per_corpus_extracted: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut total_extracted: usize = 0;
    let mut total_entities: usize = 0;
    let mut total_relationships: usize = 0;
    let mut total_failed: usize = 0;

    for (idx, (extractor, per_budget)) in extractors.into_iter().zip(allocated.iter()).enumerate() {
        if *per_budget == 0 {
            continue;
        }
        match extractor.extract_pending(*per_budget).await {
            Ok(stats) => {
                let corpus_key = corpora_roots[idx].display().to_string();
                per_corpus_extracted.insert(corpus_key, stats.docs_extracted as u32);
                total_extracted += stats.docs_extracted;
                total_entities += stats.total_entities;
                total_relationships += stats.total_relationships;
                total_failed += stats.docs_failed;
            }
            Err(e) => warn!(
                target: "mika::otel",
                trace_id = %trace_id,
                error = %e,
                agent_id = %agent_id,
                corpus_index = idx,
                event = "kg_extraction_tick.error",
                "extraction failed for corpus in tick"
            ),
        }
    }

    let per_corpus_extracted_json =
        serde_json::to_string(&per_corpus_extracted).unwrap_or_default();
    info!(
        target: "mika::otel",
        trace_id = %trace_id,
        agent_id = %agent_id,
        total_pending = total_pending,
        total_docs_extracted = total_extracted,
        total_docs_failed = total_failed,
        total_entities = total_entities,
        total_relationships = total_relationships,
        per_corpus_extracted = %per_corpus_extracted_json,
        event = "kg_extraction_tick.complete",
    );
}

/// Resolution phase of the periodic tick (#906).
async fn tick_resolution(
    agent_id: &str,
    db: &AsyncDatabase,
    llm: &Option<Arc<dyn LlmProvider>>,
    docs_root_hashes: &[String],
    budget: u32,
    trace_id: &str,
) {
    let resolver = SubjectEntityResolver::new(
        db.clone(),
        llm.clone(),
        docs_root_hashes.to_vec(),
        Some(trace_id),
    );

    // Count pending before resolution for observability.
    let pending_before = match resolver.count_pending().await {
        Ok(count) => Some(count),
        Err(e) => {
            warn!(
                target: "mika::otel",
                trace_id = %trace_id,
                agent_id = %agent_id,
                error = %e,
                event = "kg_resolver_tick.error",
                "failed to count pending entities"
            );
            None
        }
    };

    info!(
        target: "mika::otel",
        trace_id = %trace_id,
        agent_id = %agent_id,
        pending_before = pending_before,
        event = "kg_resolver_tick.start",
    );

    match resolver.resolve_pending(budget).await {
        Ok(stats) => {
            // All outcomes that write a kg_resolutions_log row remove entities
            // from the pending set — not just matched_exact + matched_llm.
            let resolved_in_tick = stats.matched_exact
                + stats.matched_llm
                + stats.no_match
                + stats.skipped_discovered
                + stats.skipped_no_llm
                + stats.errors;
            let pending_after = pending_before.map(|b| b.saturating_sub(resolved_in_tick as u64));
            let per_corpus_attempted =
                serde_json::to_string(&stats.per_corpus_attempted).unwrap_or_default();
            info!(
                target: "mika::otel",
                trace_id = %trace_id,
                agent_id = %agent_id,
                pending_before = pending_before,
                resolved_in_tick = resolved_in_tick,
                pending_after = pending_after,
                aborted_budget = stats.aborted_budget,
                llm_calls = stats.llm_calls,
                matched_exact = stats.matched_exact,
                matched_llm = stats.matched_llm,
                no_match = stats.no_match,
                reattempted_no_match = stats.reattempted_no_match,
                duration_ms = stats.duration_ms,
                per_corpus_attempted = %per_corpus_attempted,
                event = "kg_resolver_tick.complete",
            );
        }
        Err(e) => {
            warn!(
                target: "mika::otel",
                trace_id = %trace_id,
                agent_id = %agent_id,
                error = %e,
                event = "kg_resolver_tick.error",
            );
        }
    }
}

/// Coverage reporting phase (#1052).
///
/// Queries per-corpus extraction coverage and emits structured log events
/// so operators can monitor convergence without manual SQL queries.
async fn tick_coverage(
    agent_id: &str,
    db: &AsyncDatabase,
    llm: &Arc<dyn LlmProvider>,
    corpora_roots: &[PathBuf],
    trace_id: &str,
) {
    let mut coverage_map: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();

    for docs_root in corpora_roots {
        let extractor =
            SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), Some(trace_id));
        match extractor.coverage_report().await {
            Ok(cov) => {
                coverage_map.insert(
                    cov.docs_root_hash.clone(),
                    serde_json::json!({
                        "total": cov.total_docs,
                        "extracted": cov.extracted_docs,
                        "null_hash": cov.null_hash_docs,
                        "pct": (cov.coverage_pct * 10.0).round() / 10.0,
                    }),
                );
            }
            Err(e) => {
                warn!(
                    target: "mika::otel",
                    trace_id = %trace_id,
                    agent_id = %agent_id,
                    docs_root = %docs_root.display(),
                    error = %e,
                    event = "kg_extraction_coverage.error",
                    "failed to compute extraction coverage"
                );
            }
        }
    }

    if !coverage_map.is_empty() {
        let coverage_json = serde_json::to_string(&coverage_map).unwrap_or_default();
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            per_corpus_coverage = %coverage_json,
            event = "kg_extraction_coverage",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that `spawn_resolver_tick_task` with a disabled KG config
    /// exits immediately and produces a completed handle.
    #[tokio::test]
    async fn test_disabled_agent_exits_immediately() {
        use crate::kg::config::DisabledReason;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(tmp.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "test-agent");

        let kg_config = KgAgentConfig::Disabled {
            reason: DisabledReason::OperatorOptOut,
        };

        let handle = spawn_resolver_tick_task(
            "test-agent".to_string(),
            async_db,
            None, // extraction_llm
            None, // resolution_llm
            &kg_config,
            500,
            Some(1), // 1-second interval (won't fire since task exits)
        );

        // Task should complete quickly since KG is disabled.
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "disabled-agent task should complete quickly"
        );
    }

    /// Test that aborting the task handle cancels cleanly.
    #[tokio::test]
    async fn test_abort_cancels_cleanly() {
        use crate::kg::config::CorpusConfig;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(tmp.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "test-agent");

        let kg_config = KgAgentConfig::Enabled {
            corpora: vec![CorpusConfig {
                docs_root: PathBuf::from("/nonexistent"),
                docs_root_hash: "abcdef1234567890".to_string(),
            }],
        };

        let handle = spawn_resolver_tick_task(
            "test-agent".to_string(),
            async_db,
            None, // extraction_llm
            None, // resolution_llm
            &kg_config,
            500,
            Some(3600), // long interval so we can abort before it fires
        );

        handle.abort();
        let result = handle.await;
        assert!(result.is_err(), "aborted task should return JoinError");
        assert!(
            result.unwrap_err().is_cancelled(),
            "error should be cancellation"
        );
    }
}
