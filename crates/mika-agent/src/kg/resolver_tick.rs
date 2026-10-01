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
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Tick interval in seconds. Hard-coded for v1 (#906).
/// Future tunable: `MIKA_KG_RESOLVER_TICK_INTERVAL_SECS`.
const RESOLVER_TICK_INTERVAL_SECS: u64 = 30 * 60;

/// Le motif porté par la ligne de complétion d'une phase désarmée (mika#1833).
///
/// **Format de fil** : il atterrit dans le journal que l'opérateur filtre
/// (`jq 'select(.skipped_reason == "zero_budget")'`), donc deux orthographes
/// couperaient une population en deux sans le dire. Un seul site de
/// définition, épinglé par test.
const ZERO_BUDGET_SKIP_REASON: &str = "zero_budget";

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
/// * `global_home` — Home **global** (`~/.mika`), d'où le tick lit la
///   sentinelle d'arrêt à chaud [`auto_pull_stop::KG_TICK_SCAN`] (mika#1833).
///   Un paramètre, jamais une `Option` : un home absent serait un retour
///   silencieux au comportement d'avant le correctif.
/// * `interval_secs` — Tick interval override (for testing). Pass `None` to
///   use the default 30-minute interval.
#[allow(clippy::too_many_arguments)]
pub fn spawn_resolver_tick_task(
    agent_id: String,
    db: AsyncDatabase,
    extraction_llm: Option<Arc<dyn LlmProvider>>,
    resolution_llm: Option<Arc<dyn LlmProvider>>,
    kg_config: &KgAgentConfig,
    budget: u32,
    global_home: PathBuf,
    interval_secs: Option<u64>,
    cancel: CancellationToken,
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

        // Dernier état connu de la sentinelle, pour n'écrire une ligne
        // d'audit que sur **transition** (mika#1833, doctrine mika#2131 :
        // l'information durable est « le STOP a été armé à telle heure »,
        // pas qu'il l'était encore à 14 h 32 — la vivacité est le rôle de la
        // ligne INFO par tick). Perdu au redémarrage, à dessein : un process
        // neuf re-photographie l'état qu'il trouve.
        let stop_armed = std::sync::atomic::AtomicBool::new(false);

        loop {
            // Cooperative cancellation: exit between ticks on SIGTERM (#802).
            tokio::select! {
                _ = interval.tick() => {}
                _ = cancel.cancelled() => {
                    info!(
                        agent_id = %agent_id,
                        event = "kg_tick_cancelled",
                        "KG resolver tick cancelled (graceful shutdown)"
                    );
                    return;
                }
            }
            // Double-check before starting a potentially long batch.
            if cancel.is_cancelled() {
                return;
            }
            tick_body(
                &agent_id,
                &db,
                &extraction_llm,
                &resolution_llm,
                &corpora_roots,
                &docs_root_hashes,
                budget,
                &global_home,
                &stop_armed,
                &cancel,
            )
            .await;
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn tick_body(
    agent_id: &str,
    db: &AsyncDatabase,
    extraction_llm: &Option<Arc<dyn LlmProvider>>,
    resolution_llm: &Option<Arc<dyn LlmProvider>>,
    corpora_roots: &[PathBuf],
    docs_root_hashes: &[String],
    budget: u32,
    global_home: &std::path::Path,
    stop_armed: &std::sync::atomic::AtomicBool,
    cancel: &CancellationToken,
) {
    use std::sync::atomic::Ordering;

    let trace_id = mika_common::trace::generate_trace_id();

    // mika#1833 — STOP à chaud, lu **en tête du corps**, avant toute requête
    // et avant la résolution du moindre modèle. Le geste opérateur (`touch` /
    // `rm` du fichier sentinelle) est documenté dans le `CLAUDE.md` racine :
    // le chemin littéral est réservé à `auto_pull_stop.rs`, y compris en
    // commentaire, et sa garde structurelle est délibérément littérale.
    // Effectif au tick suivant (≤ 30 min), sans redémarrage et sans édition
    // d'identité.
    //
    // Une ligne INFO par tick court-circuité, à dessein : pour un
    // interrupteur, la vivacité **est** l'information (doctrine mika#2329,
    // Signal P de mika#2156). Poser le fichier et ne pas voir la ligne dans
    // les 30 minutes est un signal immédiat que le lecteur regarde ailleurs
    // que là où l'opérateur écrit. La ligne d'audit, elle, n'est écrite que
    // sur transition.
    //
    // **Aucune row n'est touchée** : le tick continue de tourner, c'est son
    // corps qui rend la main.
    let stopped =
        crate::auto_pull_stop::is_stopped(global_home, crate::auto_pull_stop::KG_TICK_SCAN);
    let was_armed = stop_armed.swap(stopped, Ordering::Relaxed);
    if stopped {
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            stop_file = %crate::auto_pull_stop::stop_file_path(
                global_home,
                crate::auto_pull_stop::KG_TICK_SCAN,
            )
            .display(),
            event = "kg_tick_stop_armed",
            "KG ingestion tick short-circuited by the stop sentinel — no row touched"
        );
        if !was_armed {
            record_stop_transition(agent_id, db, global_home, "armed").await;
        }
        return;
    }
    if was_armed {
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            event = "kg_tick_stop_lifted",
            "KG ingestion tick resumed — stop sentinel removed"
        );
        record_stop_transition(agent_id, db, global_home, "lifted").await;
    }

    // --- Phase 1: Extraction (#1052) ---
    // Run extraction before resolution so newly-extracted entities are
    // immediately available for resolution in the same tick.
    if let Some(ext_llm) = extraction_llm {
        tick_extraction(
            agent_id,
            db,
            ext_llm,
            corpora_roots,
            budget,
            &trace_id,
            cancel,
        )
        .await;
    }

    // Graceful shutdown: skip remaining phases if cancelled (#802).
    if cancel.is_cancelled() {
        return;
    }

    // --- Phase 2: Resolution (#906) ---
    tick_resolution(
        agent_id,
        db,
        resolution_llm,
        docs_root_hashes,
        budget,
        &trace_id,
        cancel,
    )
    .await;

    // Graceful shutdown: skip coverage report if cancelled (#802).
    if cancel.is_cancelled() {
        return;
    }

    // --- Phase 3: Coverage report (#1052) ---
    // Log per-corpus extraction coverage after both phases complete.
    // Only runs when extraction LLM is configured (otherwise no extractors
    // to query coverage from).
    if let Some(ext_llm) = extraction_llm {
        tick_coverage(agent_id, db, ext_llm, corpora_roots, &trace_id).await;
    }
}

/// Une transition de la sentinelle d'arrêt du tick KG, écrite en
/// `audit_events` (mika#1833).
///
/// Un `tool_name` **distinct** de ceux des deux autres scans
/// (`auto_pull_stop`, `worktree_reap_stop`) : c'est ce qui laisse l'opérateur
/// compter trois populations séparément — motif `phantom_aged_out` /
/// `phantom_sweep_spared` (mika#2156). Une ligne par transition, jamais par
/// tick (doctrine mika#2131).
///
/// Fire-and-forget : une écriture d'audit qui échoue ne doit pas pouvoir
/// changer ce que la sentinelle décide.
async fn record_stop_transition(
    agent_id: &str,
    db: &AsyncDatabase,
    global_home: &std::path::Path,
    state: &str,
) {
    if let Err(e) = db
        .log_audit_event(
            // Underscores, comme le `tool_name` — et surtout pas le littéral
            // du chemin du fichier, que la garde structurelle de
            // `auto_pull_stop.rs` réserve à ce module-là.
            &format!("kg_tick_stop-{agent_id}"),
            "kg_tick_stop",
            "scan:kg_tick",
            None,
            Some(state),
            Some(&format!(
                "fichier sentinelle : {}",
                crate::auto_pull_stop::stop_file_path(
                    global_home,
                    crate::auto_pull_stop::KG_TICK_SCAN,
                )
                .display()
            )),
            None,
        )
        .await
    {
        warn!(
            agent_id = %agent_id,
            state = %state,
            error = %e,
            event = "kg_tick_stop_audit_failed",
            "failed to record KG tick stop transition"
        );
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
    cancel: &CancellationToken,
) {
    // mika#1833 — court-circuit **avant** `count_pending_docs`, qui est là où
    // le coût vit. La ligne de complétion est conservée : sans elle, « le tick
    // tourne et ne fait rien » et « le tick ne tourne pas » rendraient des
    // octets identiques (classe mika#2205).
    if crate::kg::budget::phase_is_disabled(budget) {
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            total_pending = Option::<u32>::None,
            skipped_reason = ZERO_BUDGET_SKIP_REASON,
            event = "kg_extraction_tick.complete",
            "extraction phase disabled by a zero budget — no counting query issued"
        );
        return;
    }

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
        // Graceful shutdown: skip remaining corpora (#802).
        if cancel.is_cancelled() {
            break;
        }
        if *per_budget == 0 {
            continue;
        }
        let extractor = extractor.with_cancel_token(cancel.child_token());
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
    cancel: &CancellationToken,
) {
    // mika#1833 — même court-circuit que la phase d'extraction, et pour la
    // même raison : `count_pending` porte la sous-requête corrélée sur
    // `kg_chunk_subjects` mesurée à 26-48 s par tick sous budget nul.
    //
    // `pending_before` est **absent**, jamais `0` : le comptage n'a pas eu
    // lieu, il n'a pas rendu zéro (motif mika#2331 — `null` n'est jamais `0`).
    if crate::kg::budget::phase_is_disabled(budget) {
        info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            pending_before = Option::<u64>::None,
            skipped_reason = ZERO_BUDGET_SKIP_REASON,
            event = "kg_resolver_tick.complete",
            "resolution phase disabled by a zero budget — no counting query issued"
        );
        return;
    }

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        llm.clone(),
        docs_root_hashes.to_vec(),
        Some(trace_id),
    )
    .with_cancel_token(cancel.child_token());

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

            // --- 7-day no_match rate alert (#1077) ---
            // After each resolution tick, compute the rolling 7-day no_match
            // rate. Emit WARN when the agent-wide rate exceeds 60% and the
            // sample size is sufficient (>= 50 attempted) to avoid false alarms.
            check_no_match_rate(agent_id, db, docs_root_hashes, trace_id).await;
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

/// Minimum attempted resolutions required before the no_match rate alert
/// fires. Prevents false alarms on cold-start or newly-provisioned agents.
const NO_MATCH_RATE_MIN_SAMPLE: u64 = 50;

/// Threshold above which the 7-day no_match rate triggers a WARN log.
const NO_MATCH_RATE_THRESHOLD: f64 = 0.60;

/// Rolling window in days for the no_match rate computation.
const NO_MATCH_RATE_WINDOW_DAYS: u32 = 7;

/// Check the rolling 7-day no_match rate after each resolution tick (#1077).
///
/// Computes the agent-wide rate first. If it exceeds the threshold (>60%)
/// with a sufficient sample size (>=50 attempted), emits a WARN log with
/// per-corpus breakdown so operators can identify which corpus is driving
/// the rate.
async fn check_no_match_rate(
    agent_id: &str,
    db: &AsyncDatabase,
    docs_root_hashes: &[String],
    trace_id: &str,
) {
    let aid = agent_id.to_string();
    let agent_wide = db
        .with_db(move |db| db.kg_resolution_outcome_stats(&aid, None, NO_MATCH_RATE_WINDOW_DAYS))
        .await;

    let stats = match agent_wide {
        Ok(s) => s,
        Err(e) => {
            warn!(
                target: "mika::otel",
                trace_id = %trace_id,
                agent_id = %agent_id,
                error = %e,
                event = "kg_no_match_rate_check.error",
                "failed to compute 7-day no_match rate"
            );
            return;
        }
    };

    let rate = stats.no_match_rate();
    if rate <= NO_MATCH_RATE_THRESHOLD || stats.attempted < NO_MATCH_RATE_MIN_SAMPLE {
        return;
    }

    // Agent-wide threshold exceeded — compute per-corpus breakdown for the
    // WARN log so operators can see which corpus is contributing.
    let mut per_corpus = serde_json::Map::new();
    for hash in docs_root_hashes {
        let aid = agent_id.to_string();
        let h = hash.clone();
        let corpus_stats = db
            .with_db(move |db| {
                db.kg_resolution_outcome_stats(&aid, Some(&h), NO_MATCH_RATE_WINDOW_DAYS)
            })
            .await;
        if let Ok(cs) = corpus_stats {
            per_corpus.insert(
                hash.clone(),
                serde_json::json!({
                    "no_match_count": cs.no_match,
                    "total": cs.attempted,
                    "rate": (cs.no_match_rate() * 1000.0).round() / 1000.0,
                }),
            );
        }
    }
    let per_corpus_json =
        serde_json::to_string(&serde_json::Value::Object(per_corpus)).unwrap_or_default();

    warn!(
        target: "mika::otel",
        trace_id = %trace_id,
        agent_id = %agent_id,
        no_match_rate = rate,
        no_match_count = stats.no_match,
        attempted_count = stats.attempted,
        window_days = NO_MATCH_RATE_WINDOW_DAYS,
        per_corpus = %per_corpus_json,
        event = "kg_no_match_rate_high",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc as StdArc, Mutex};

    // -- capture tracing (même forme que `tests/llm_call_attempt_2342.rs`) --

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        fields: HashMap<String, String>,
    }

    struct CapturingLayer {
        events: StdArc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = HashMap::new();
            let mut visitor = FieldVisitor(&mut fields);
            event.record(&mut visitor);
            if let Ok(mut events) = self.events.lock() {
                events.push(CapturedEvent { fields });
            }
        }
    }

    struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for FieldVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    fn capture() -> (
        tracing::subscriber::DefaultGuard,
        StdArc<Mutex<Vec<CapturedEvent>>>,
    ) {
        use tracing_subscriber::layer::SubscriberExt;
        let events = StdArc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(CapturingLayer {
            events: StdArc::clone(&events),
        });
        let guard = tracing::subscriber::set_default(subscriber);
        (guard, events)
    }

    fn events_named(events: &StdArc<Mutex<Vec<CapturedEvent>>>, name: &str) -> Vec<CapturedEvent> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.fields.get("event").map(String::as_str) == Some(name))
            .cloned()
            .collect()
    }

    /// Une base neuve plus un home global neuf, pour les tests de tick.
    ///
    /// L'agent est inséré : `audit_events.agent_id` porte une clé étrangère
    /// vers `agents(id)`, donc sans lui la ligne d'audit du STOP échouerait —
    /// et le test lirait « aucune transition écrite » alors que le défaut
    /// serait dans la fixture.
    fn tick_fixture() -> (tempfile::NamedTempFile, tempfile::TempDir, AsyncDatabase) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(tmp.path()).unwrap();
        db.conn
            .execute(
                "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES (?1, ?1, '')",
                rusqlite::params!["test-agent"],
            )
            .expect("seed agent row");
        let async_db = AsyncDatabase::new_with_agent(db, "test-agent");
        let home = tempfile::tempdir().unwrap();
        (tmp, home, async_db)
    }

    /// V4 — le tick désarmé émet quand même sa ligne de complétion
    /// (mika#1833 R2).
    ///
    /// Sans elle, R2 rendrait le tick **muet** et le contrôle positif serait
    /// perdu : « le tick tourne et ne fait rien » et « le tick ne tourne pas »
    /// rendraient des octets identiques (classe mika#2205). Et
    /// `pending_before` doit être **absent**, jamais `0` — le comptage n'a pas
    /// eu lieu, il n'a pas rendu zéro (motif mika#2331).
    #[tokio::test]
    async fn mika1833_zero_budget_tick_still_emits_its_completion_line() {
        let (_tmp, home, db) = tick_fixture();
        let (_guard, events) = capture();

        tick_body(
            "test-agent",
            &db,
            &None,
            &None,
            &[PathBuf::from("/nonexistent")],
            &["abcdef1234567890".to_string()],
            0, // budget nul
            home.path(),
            &std::sync::atomic::AtomicBool::new(false),
            &CancellationToken::new(),
        )
        .await;

        let completions = events_named(&events, "kg_resolver_tick.complete");
        assert_eq!(
            completions.len(),
            1,
            "le tick désarmé doit rester audible — une ligne de complétion, \
             exactement"
        );
        assert_eq!(
            completions[0]
                .fields
                .get("skipped_reason")
                .map(String::as_str),
            Some(ZERO_BUDGET_SKIP_REASON),
            "…et dire pourquoi il n'a rien fait"
        );
        // `tracing` n'enregistre AUCUN champ pour un `Option::None`, donc la
        // clé est absente de l'événement et sort `null` en JSON. C'est très
        // exactement ce que R2 demande : `pending_before: null`, jamais `0` —
        // le comptage n'a pas eu lieu, il n'a pas rendu zéro (mika#2331).
        assert!(
            !completions[0].fields.contains_key("pending_before"),
            "`pending_before` doit être ABSENT (donc `null` en JSON), jamais \
             `0` : annoncer zéro serait affirmer un comptage qui n'a pas eu \
             lieu. Champs observés : {:?}",
            completions[0].fields.keys().collect::<Vec<_>>()
        );
    }

    /// V5 — la sentinelle court-circuite le tick, et son retrait le reprend
    /// (mika#1833 R5).
    ///
    /// Le contrôle négatif — la seconde moitié — est ce qui distingue « la
    /// sentinelle décide » de « le tick est mort ».
    #[tokio::test]
    async fn mika1833_the_stop_sentinel_short_circuits_the_tick() {
        let (_tmp, home, db) = tick_fixture();
        let stop_path =
            crate::auto_pull_stop::stop_file_path(home.path(), crate::auto_pull_stop::KG_TICK_SCAN);
        std::fs::create_dir_all(stop_path.parent().unwrap()).unwrap();
        std::fs::write(&stop_path, "").unwrap();

        let armed = std::sync::atomic::AtomicBool::new(false);

        {
            let (_guard, events) = capture();
            tick_body(
                "test-agent",
                &db,
                &None,
                &None,
                &[PathBuf::from("/nonexistent")],
                &["abcdef1234567890".to_string()],
                500, // budget non nul : seule la sentinelle peut couper
                home.path(),
                &armed,
                &CancellationToken::new(),
            )
            .await;

            assert_eq!(
                events_named(&events, "kg_tick_stop_armed").len(),
                1,
                "un tick court-circuité doit le dire — pour un interrupteur, \
                 la vivacité EST l'information (mika#2329)"
            );
            assert!(
                events_named(&events, "kg_resolver_tick.start").is_empty(),
                "…et ne doit avoir émis aucune requête de résolution"
            );
        }

        // La ligne d'audit n'est écrite qu'une fois, sur la transition.
        assert_eq!(
            stop_transitions(&db).await,
            vec!["armed".to_string()],
            "une ligne d'audit par transition"
        );

        // Second tick, sentinelle TOUJOURS posée, même `armed` : la ligne INFO
        // se répète (la vivacité est l'information), la ligne d'audit NON.
        // Sans ce second tick, retirer la garde `if !was_armed` laissait ce
        // test vert — un seul tick ne distingue pas « par transition » de
        // « par tick ».
        {
            let (_guard, events) = capture();
            tick_body(
                "test-agent",
                &db,
                &None,
                &None,
                &[PathBuf::from("/nonexistent")],
                &["abcdef1234567890".to_string()],
                500,
                home.path(),
                &armed,
                &CancellationToken::new(),
            )
            .await;
            assert_eq!(
                events_named(&events, "kg_tick_stop_armed").len(),
                1,
                "chaque tick court-circuité le dit"
            );
        }
        assert_eq!(
            stop_transitions(&db).await,
            vec!["armed".to_string()],
            "une ligne d'audit par transition, jamais par tick : un second tick \
             armé ne doit rien écrire"
        );

        // Contrôle négatif : la sentinelle retirée, le tick reprend.
        std::fs::remove_file(&stop_path).unwrap();
        let (_guard, events) = capture();
        tick_body(
            "test-agent",
            &db,
            &None,
            &None,
            &[PathBuf::from("/nonexistent")],
            &["abcdef1234567890".to_string()],
            500,
            home.path(),
            &armed,
            &CancellationToken::new(),
        )
        .await;

        assert!(
            events_named(&events, "kg_tick_stop_armed").is_empty(),
            "sentinelle retirée : plus de court-circuit"
        );
        assert_eq!(
            events_named(&events, "kg_tick_stop_lifted").len(),
            1,
            "…et la levée est dite, une fois"
        );
        assert_eq!(
            events_named(&events, "kg_resolver_tick.start").len(),
            1,
            "le tick reprend réellement son travail — sans ce contrôle, « la \
             sentinelle décide » serait indistinguable de « le tick est mort »"
        );
        assert_eq!(
            stop_transitions(&db).await,
            vec!["armed".to_string(), "lifted".to_string()],
            "la levée est une transition, donc une seconde ligne d'audit, et la \
             seule"
        );
    }

    /// Les `after_value` des lignes d'audit `kg_tick_stop`, dans l'ordre
    /// d'écriture.
    async fn stop_transitions(db: &AsyncDatabase) -> Vec<String> {
        db.with_db(|db| {
            let mut stmt = db.conn.prepare(
                "SELECT after_value FROM audit_events \
                 WHERE tool_name = 'kg_tick_stop' ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
        .expect("audit query")
    }

    /// Test that `spawn_resolver_tick_task` with a disabled KG config
    /// exits immediately and produces a completed handle.
    #[tokio::test]
    async fn test_disabled_agent_exits_immediately() {
        use crate::kg::config::DisabledReason;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(tmp.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "test-agent");
        let home = tempfile::tempdir().unwrap();

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
            home.path().to_path_buf(),
            Some(1), // 1-second interval (won't fire since task exits)
            CancellationToken::new(),
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
        let home = tempfile::tempdir().unwrap();

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
            home.path().to_path_buf(),
            Some(3600), // long interval so we can abort before it fires
            CancellationToken::new(),
        );

        handle.abort();
        let result = handle.await;
        assert!(result.is_err(), "aborted task should return JoinError");
        assert!(
            result.unwrap_err().is_cancelled(),
            "error should be cancellation"
        );
    }

    /// Test that cancelling the token exits the tick loop cleanly (#802).
    #[tokio::test]
    async fn test_cancellation_token_exits_cleanly() {
        use crate::kg::config::CorpusConfig;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::Database::open(tmp.path()).unwrap();
        let async_db = AsyncDatabase::new_with_agent(db, "test-agent");
        let home = tempfile::tempdir().unwrap();

        let kg_config = KgAgentConfig::Enabled {
            corpora: vec![CorpusConfig {
                docs_root: PathBuf::from("/nonexistent"),
                docs_root_hash: "abcdef1234567890".to_string(),
            }],
        };

        let cancel = CancellationToken::new();
        let handle = spawn_resolver_tick_task(
            "test-agent".to_string(),
            async_db,
            None,
            None,
            &kg_config,
            500,
            home.path().to_path_buf(),
            Some(3600), // long interval
            cancel.clone(),
        );

        // Cancel the token — the task should exit cleanly (Ok, not Err).
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "task should complete within timeout");
        assert!(
            result.unwrap().is_ok(),
            "cancelled task should exit cleanly (Ok), not via abort (Err)"
        );
    }
}
