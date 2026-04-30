//! Periodic resolver tick for draining the Stage-2 entity resolution backlog.
//!
//! Decouples resolver drain rate from restart cadence (#906). Runs
//! `resolve_pending(budget)` every 30 minutes per KG-enabled agent at the
//! existing `MIKA_KG_BATCH_BUDGET` (default 500), preserving the
//! "no silent multi-thousand-call bursts" invariant from #757.
//!
//! The tick joins the startup background spawn and compound-hook synchronous
//! spawn as the third execution context for `resolve_pending`. All three use
//! `kg_resolutions_log UNIQUE(agent_id, subject_entity_id)` as the
//! deduplication mechanism, so races result in one fast no-op.
//!
//! Pattern follows `server::checkpoint::spawn_dashboard_checkpoint_task()`:
//! interval + fail-open (log-and-skip) + lifecycle tied to tokio runtime drop.

use crate::async_db::AsyncDatabase;
use crate::kg::config::KgAgentConfig;
use crate::kg::entity_resolver::SubjectEntityResolver;
use mika_common::llm::LlmProvider;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Tick interval in seconds. Hard-coded for v1 (#906).
/// Future tunable: `MIKA_KG_RESOLVER_TICK_INTERVAL_SECS`.
const RESOLVER_TICK_INTERVAL_SECS: u64 = 30 * 60;

/// Spawns a background tokio task that runs `resolve_pending(budget)` every
/// [`RESOLVER_TICK_INTERVAL_SECS`] for a single KG-enabled agent.
///
/// The first immediate fire is skipped so the startup background resolver
/// handles the immediate post-restart drain. Subsequent ticks fire on the
/// 30-min cadence.
///
/// # Arguments
///
/// * `agent_id` — Agent name (for logging and DB scoping).
/// * `db` — Async database handle carrying the agent_id.
/// * `llm` — Optional resolution LLM provider. `None` = exact-match-only.
/// * `kg_config` — Agent's KG configuration. If `Disabled`, the task exits
///   immediately.
/// * `budget` — Per-batch Stage-2 LLM call cap (from `MIKA_KG_BATCH_BUDGET`).
/// * `interval_secs` — Tick interval override (for testing). Pass `None` to
///   use the default 30-minute interval.
pub fn spawn_resolver_tick_task(
    agent_id: String,
    db: AsyncDatabase,
    llm: Option<Arc<dyn LlmProvider>>,
    kg_config: &KgAgentConfig,
    budget: u32,
    interval_secs: Option<u64>,
) -> tokio::task::JoinHandle<()> {
    let docs_root_hashes = match kg_config {
        KgAgentConfig::Enabled { corpora } => {
            corpora.iter().map(|c| c.docs_root_hash.clone()).collect()
        }
        KgAgentConfig::Disabled { .. } => Vec::new(),
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
            tick_body(&agent_id, &db, &llm, &docs_root_hashes, budget).await;
        }
    })
}

async fn tick_body(
    agent_id: &str,
    db: &AsyncDatabase,
    llm: &Option<Arc<dyn LlmProvider>>,
    docs_root_hashes: &[String],
    budget: u32,
) {
    let trace_id = mika_common::trace::generate_trace_id();

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        llm.clone(),
        docs_root_hashes.to_vec(),
        Some(&trace_id),
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
                duration_ms = stats.duration_ms,
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
            None,
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
        use std::path::PathBuf;

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
            None,
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
