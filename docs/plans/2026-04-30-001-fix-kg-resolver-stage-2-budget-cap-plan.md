---
title: "fix(kg/resolver): add periodic resolver tick to drain Stage-2 backlog without restart"
type: fix
status: active
date: 2026-04-30
---

# fix(kg/resolver): add periodic resolver tick to drain Stage-2 backlog without restart

## Overview

`MIKA_KG_BATCH_BUDGET=500` per agent caps Stage-2 LLM disambiguation. Stage-2 currently runs in only two contexts (startup background spawn + compound-hook synchronous spawn), so steady-state drain is gated by restart cadence — and in production with infrequent restarts, drain rate falls below new-extraction rate. The fix is **single-shape**: add a periodic in-process resolver tick that runs `resolve_pending` every 30 minutes per KG-enabled agent at the existing 500-call budget. This decouples drain rate from restart cadence without touching the budget itself, preserving mika#757's "no silent multi-thousand-call bursts" invariant.

## Problem Frame

mika's KG resolver runs `resolve_pending(budget)` only at startup (background `tokio::spawn` per agent) and after each compound-extraction hook (synchronous spawn via `IngestionOrchestrator`). Per `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Entity Resolver, those are the two and only execution contexts today. Each invocation is bounded by `MIKA_KG_BATCH_BUDGET` (default 500) on Stage-2 LLM calls. Stage-1 exact matches are budget-free.

Empirical state (verified 2026-04-30 against `~/.mika/data/mika.db`):

- mika-arch primary corpus cumulative: 39 `matched_exact` + 1999 `matched_llm` + 9479 `no_match` = ~11.5K resolved; ~17K still pending.
- Most recent batch (resolved_at > 2026-04-29): mika-arch produced `matched_exact=3, matched_llm=449, no_match=1548` — totaling 1997 Stage-2 attempts, of which 449 succeeded.
- Cost framing per `mika/CLAUDE.md` Signal D: ~$0.0001/call at OpenRouter cheap-tier.
- Per `feedback_qa_provider_perf` and `mika/CLAUDE.md` § Environment Variables, default 500 was deliberately set in mika#757 as a startup-burst guard following a $40–60 incident on 2026-04-23. The mika#757 AC reads (verbatim, gh_read 2026-04-30): *"Cap LLM calls per extraction/resolution batch (configurable via env, default e.g. 500). On overflow: log WARN with exact count, abort the batch, leave remaining work for subsequent deferred runs. **No silent multi-thousand-call bursts.**"*

The bottleneck is **execution cadence**, not budget. Production restarts are infrequent; new extraction continuously surfaces subjects via the compound hook; resolver fires opportunistically. This produces a drain-rate-vs-extraction-rate inversion that grows the backlog over time, with no operator path to drain except restart.

mika#757's AC explicitly anticipates this scenario: *"leave remaining work for subsequent deferred runs."* The tick is exactly that subsequent-deferred-run mechanism; it inherits #757's framing rather than relaxing it.

## Requirements Trace

- **R1.** Drain rate ≥ new-extraction rate over a 3-batch sustained window for mika-arch primary corpus (per ticket AC).
- **R2.** `kg_budget_exhausted` events become rare (per ticket AC). Operationally: a healthy steady-state restart should produce 0 `kg_budget_exhausted` lines under the existing default; only large-backlog scenarios may cap.
- **R3.** mika-arch primary corpus pending count trends to 0 across restarts and ticks (per Signal C in `mika/CLAUDE.md` § Post-restart safety check #757).
- **R4.** Cost predictability: per-tick cost remains bounded by the existing budget; no unbounded LLM spend.
- **R5.** Operator escape hatch preserved: `MIKA_KG_BATCH_BUDGET` env var continues to override for one-off backlog-drain scenarios.
- **R6.** No regression on Stage-1 exact-match path (verified functional in mika#875 disconfirmation).
- **R7.** Periodic tick must coexist with the existing compound-hook synchronous spawn and the startup background spawn — three execution contexts for `resolve_pending` must coexist without double-resolution races.
- **R8.** Burst invariant preserved (mika#757): no per-tick or per-restart event exceeds the existing 500-call default. Throughput improvement comes from cadence, not burst size.

## Scope Boundaries

- **In scope:**
  - New periodic resolver tick on `tokio::time::interval` per agent (Unit 1).
  - Tick-side observability: structured log events at INFO level (Unit 1).
  - Documentation updates: `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Entity Resolver (third execution context), § Post-restart safety check (new Signal E for tick drain), `mika/CLAUDE.md` § Environment Variables (note tick-aware drain) (Unit 2).
  - Integration test exercising the relative-rate property (drain ≥ extraction over multiple ticks) (Unit 1's test).
- **Out of scope:**
  - **Default `MIKA_KG_BATCH_BUDGET` value.** Stays at 500 per mika#757 invariant ("no silent multi-thousand-call bursts"). Operators can raise via env for one-off drains. *Architect first-pass F1 explicitly required this scope decision.*
  - Tunable `MIKA_KG_RESOLVER_TICK_INTERVAL_SECS` env var. Deferred to follow-up if operator scenario emerges, matching `MIKA_KG_BATCH_BUDGET`'s lazy-introduction precedent (#757 added the env var only after the incident).
  - Stage-1 exact-match path (mika#875 closed-as-not-a-bug; #906 deliberately scoped to Stage-2).
  - Async budget refresh within a single `resolve_pending` call (Shape 4 from ticket — over-engineered).
  - Time-bounded loop replacement of count-bounded budget (Shape 2 — loses cost predictability that #757 deliberately added).
  - Extraction-rate concerns (logged on mika#876).
  - Secondary corpora resolution shape (logged on mika#877; #906 affects all corpora uniformly via the existing internal IN-list iteration).

## Phase 0 Pins (load-bearing source verification)

These claims back the plan's design choices and were verified before commit per `current_priorities` core memory's Phase 0 Pin pattern.

### Pin 1: `resolve_pending` short-circuits on empty pending — **CONFIRMED**

`crates/mika-agent/src/kg/entity_resolver.rs:233-251`:

```rust
pub async fn resolve_pending(&self, budget: u32) -> Result<ResolutionStats> {
    let start = Instant::now();
    let agent_id = self.db.agent_id.clone();
    let pending = self.get_pending_entities().await?;
    info!(...);
    if pending.is_empty() {
        return Ok(ResolutionStats {
            duration_ms: start.elapsed().as_millis() as u64,
            ..Default::default()
        });
    }
    // ... LLM-call loop only runs when pending is non-empty
}
```

**Implication:** Idle tick cost = 1 `get_pending_entities` SQLite query per agent per 30 min. Drained-state cost is negligible.

### Pin 2: Multi-corpus fan-out is internal via SQL IN-list — **CONFIRMED**

`crates/mika-agent/src/kg/entity_resolver.rs:152-156, 707-720`:

```rust
pub struct EntityResolver {
    // ...
    /// Shared-corpus keys for querying v27 shared-layer tables (#798: multi-corpus).
    docs_root_hashes: Vec<String>,
    // ...
}

// In get_pending_entities():
let placeholders: Vec<String> = docs_root_hashes.iter().map(|_| "?".to_string()).collect();
"SELECT ... FROM kg_subject_entities WHERE docs_root_hash IN ({}) AND id IN ({})"
```

**Implication:** A single `resolve_pending(budget)` call drains across all of mika-arch's 4 corpora via a single SQL query with IN-list parameter expansion. The tick callsite stays `resolve_pending(budget)` — no per-corpus fan-out at the call layer. Budget covers the union of corpora (matches the existing startup spawn semantics; no per-corpus division needed).

### Pin 3: Pattern reference for periodic tokio task — `checkpoint.rs::spawn_dashboard_checkpoint_task` (lifecycle, fail-open, log shape)

`crates/mika-agent/src/server/checkpoint.rs::spawn_dashboard_checkpoint_task()` runs `PRAGMA wal_checkpoint(PASSIVE)` every 60s via `tokio::time::interval`. Documented in `crates/mika-agent/CLAUDE.md` § HTTP Server. Lifecycle: tied to tokio runtime drop (no explicit shutdown hook). Body fully bounded per iteration (no held resources). Fail mode: log-and-skip via `tracing::warn!`. The resolver tick mirrors this shape.

### Pin 4: Documentation parity surface for the new periodic task

Per architect F9 sharpening, ran `grep -rln "checkpoint_task\|spawn_dashboard\|periodic.*task" docs/` and `grep -in "checkpoint_task\|background task\|tokio task\|tick" docs/runtime-structure.md` to identify documentation surfaces that enumerate background tasks. Result: `docs/runtime-structure.md` has no background-tasks section; `docs/architecture.md` mentions periodic dispatch in passing but has no canonical enumeration; the authoritative surface for periodic tokio-task documentation is `crates/mika-agent/CLAUDE.md` (§ HTTP Server documents `checkpoint_task`; § Knowledge Graph — Entity Resolver documents the resolver's two existing execution contexts). **Implication:** Unit 2's targeting of `crates/mika-agent/CLAUDE.md` covers the documentation parity requirement; no additional documentation surface needs editing.

## Context & Research

### Relevant Code and Patterns

- **Resolver entry point:** `crates/mika-agent/src/kg/entity_resolver.rs::resolve_pending(budget: u32)` (line 233). Two existing callers — startup background spawn + compound-hook synchronous spawn via `IngestionOrchestrator`.
- **Periodic tokio task pattern:** `crates/mika-agent/src/server/checkpoint.rs::spawn_dashboard_checkpoint_task()`. Same shape required (interval + fail-open + structured log event).
- **Settings field:** `MIKA_KG_BATCH_BUDGET` defined in `crates/mika-common/src/settings.rs` (or wherever `Settings` lives). Default 500 per `mika/CLAUDE.md` § Environment Variables.
- **Idempotency invariant:** `kg_resolutions_log` has `UNIQUE(agent_id, subject_entity_id)` per `crates/mika-agent/src/kg/entity_resolver.rs` doc comment. Concurrent resolution of the same entity is safe — second writer gets `INSERT OR IGNORE`.

### Institutional Learnings

- **`mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md`** — disconfirmation procedure that produced #906's framing. The DB ratios in the Problem Frame come from this procedure.
- **`mika/CLAUDE.md` § Post-restart safety check (#757)** — Signal C documents the pending-count-trend-to-zero contract; this plan's R3 commits to that signal. Adds Signal E for tick drain.
- **`mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`** — informs why Shape 5 (accept + document) was rejected: AC requires sustained drain, not better operator handling.
- **`mika/docs/solutions/best-practices/carve-out-trigger-outcome-shape-vs-causation-2026-04-30.md`** (committed on coordination branch, milestone#19 work) — informs F4: tick interval tunable env var deferred per lazy-introduction precedent, not preemptive operator-tunability.

### Downstream Consumers

- `mika kg status --agent <name>` CLI reports pending counts; users see counts trend to 0 between restarts.
- mika-arch's `query_knowledge_graph` tool depends on resolver having drained subject→domain edges. This fix is on the critical path for mika-arch architectural reasoning quality (currently degraded with 17K+ pending in primary corpus + ~0 resolutions in 3 secondary corpora).

## Key Technical Decisions

- **Default budget unchanged at 500.** Per #757's "no silent multi-thousand-call bursts" invariant. Throughput improvement via cadence (Unit 1's tick), not burst size. *Architect first-pass F1 explicitly redirected here.*
- **Periodic tick interval: 30 minutes.** Cold-path drain cadence; 5-min would be hot-path-cadence overkill, 1-hour leaves backlog growing during compound-doc-heavy work sessions. Hard-coded for v1; tunable env var deferred per #757's lazy-introduction precedent (F4).
- **One periodic tick task per agent (not one global task).** Mirrors per-agent startup background spawn. Each task uses its own KG config (per `[kg]` section in identity.toml). Disabled agents (`enabled=false`) get no tick. Multi-corpus agents (mika-arch) tick once per agent and fan out across corpora **internally** via the resolver's IN-list query (Pin 2 confirms this).
- **Tick uses the same `resolve_pending(budget)` entry point.** No new resolver entry point. Reuses budget enforcement, idempotency guards, retry taxonomy, and observability (`llm_calls` rows, `audit_events`).
- **Tick observability:** new structured log events `kg_resolver_tick.{start,complete,error}` at INFO level (`target: "mika::otel"` per existing convention) with fields `agent_id`, `pending_before`, `resolved_in_tick`, `pending_after`, `aborted_budget`, `llm_calls`. Two derived signals: (a) `pending_after == 0` confirms drain reached zero; (b) `aborted_budget == true` flags when 500-call budget is insufficient and operator may want to raise via `MIKA_KG_BATCH_BUDGET=10000` for the next restart.
- **Tick failure mode: log-and-skip.** Same C2.3 contract as the existing background spawn and `checkpoint_task`. A tick failing does NOT abort the interval; the next tick fires normally.
- **Tick lifecycle:** spawned at agent init alongside the existing startup background resolver. Tied to tokio runtime drop — no explicit shutdown hook. Body fully bounded per iteration (no DB transactions or network connections held across ticks). Matches `checkpoint_task` precedent (Pin 3).
- **First fire skipped:** `interval.tick().await` at the top of the loop (before the work body) consumes the immediate first fire so the startup background resolver handles the immediate post-restart drain. Subsequent ticks fire on the 30-min cadence.
- **Race surface (F5 ratified by architect):** `count_pending_resolutions` (read inside `resolve_pending`'s `get_pending_entities` per Pin 1) returning stale data has two failure modes: (1) under-count → tick decides "no work," skips → bounded staleness, next tick (30 min) catches up; (2) over-count → tick spends budget on rows already resolved by concurrent context → UNIQUE constraint rejects redundant inserts → wasted tick, no corruption. Both bounded and safe. No mitigation required beyond the existing UNIQUE constraint.

## Implementation Units

- [ ] **Unit 1: Add periodic resolver tick task**

  **Goal:** Decouple drain rate from restart cadence; resolver runs every 30 min per agent at the existing 500-call budget.

  **Requirements:** R1, R3, R4, R5, R7, R8

  **Dependencies:** None.

  **Files:**
  - Add: `crates/mika-agent/src/kg/resolver_tick.rs` (new module) — `spawn_resolver_tick_task(agent_id, settings, db, kg_config) -> JoinHandle<()>`.
  - Modify: `crates/mika-agent/src/kg/mod.rs` — register the new module via `pub mod resolver_tick;`.
  - Modify: agent-init path that spawns the existing startup background resolver (likely `crates/mika-agent/src/server.rs` or a per-agent init function) — call the new spawn alongside.
  - Add: `crates/mika-agent/src/kg/resolver_tick.rs` test module — integration test exercising relative-rate property.

  **Approach:**
  - New module with one public spawn function and one private tick body. Internally:
    ```rust
    pub fn spawn_resolver_tick_task(
        agent_id: String,
        settings: Arc<Settings>,
        db: AsyncDatabase,
        kg_config: KgAgentConfig,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Skip if KG disabled for this agent
            let KgAgentConfig::Enabled { docs_root_hashes, .. } = kg_config else { return };
            let interval_secs = 30 * 60;
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.tick().await; // skip first immediate fire — startup spawn covers it
            loop {
                interval.tick().await;
                tick_body(&agent_id, &settings, &db, &docs_root_hashes).await;
            }
        })
    }

    async fn tick_body(
        agent_id: &str,
        settings: &Settings,
        db: &AsyncDatabase,
        docs_root_hashes: &[String],
    ) {
        let trace_id = generate_trace_id();
        let resolver = match EntityResolver::new(db.clone(), docs_root_hashes.to_vec(), ...) {
            Ok(r) => r,
            Err(e) => { tracing::warn!(target: "mika::otel", trace_id, agent_id, error = %e, "kg_resolver_tick.error"); return; }
        };
        // pending_before is read inside resolve_pending via get_pending_entities;
        // we re-derive it here for the structured log only (one extra cheap query).
        let pending_before = resolver.count_pending().await.ok();
        tracing::info!(target: "mika::otel", trace_id, agent_id, pending_before,
            "kg_resolver_tick.start");
        match resolver.resolve_pending(settings.kg_batch_budget).await {
            Ok(stats) => {
                tracing::info!(target: "mika::otel", trace_id, agent_id,
                    pending_before, resolved_in_tick = stats.resolved,
                    pending_after = pending_before.map(|b| b.saturating_sub(stats.resolved as u64)),
                    aborted_budget = stats.aborted_budget, llm_calls = stats.llm_calls,
                    "kg_resolver_tick.complete");
            }
            Err(e) => {
                tracing::warn!(target: "mika::otel", trace_id, agent_id,
                    error = %e, "kg_resolver_tick.error");
            }
        }
    }
    ```
  - Wire into agent init at the same call site as the existing startup background resolver. The startup spawn handles immediate post-restart drain; the tick handles steady-state.
  - The 30-min interval is hard-coded. Future tunable env var (`MIKA_KG_RESOLVER_TICK_INTERVAL_SECS`) deferred (Future Work section).
  - **`count_pending` helper:** if `EntityResolver` does not already expose a public `count_pending` method, add one (just runs `SELECT COUNT(*)` against the existing pending-entities query). This is a small additive surface — no behavior change to `resolve_pending` itself. If reading the agent-init wiring reveals that the count is already exposed via stats, prefer that over adding a new method. **SQL shape (per architect F8 sharpening):** if (b), the new method's SQL mirrors `get_pending_entities`'s WHERE clause (same `docs_root_hash IN (?,?,...)` multi-corpus pattern, same `LEFT JOIN kg_resolutions_log r ON r.subject_entity_id = e.id` pending semantics) but with `SELECT COUNT(*)` projection. This pre-authors the multi-corpus semantics at the count layer to match Pin 2's IN-list pattern.

  **Patterns to follow:**
  - `crates/mika-agent/src/server/checkpoint.rs::spawn_dashboard_checkpoint_task()` — interval shape, log-event-on-each-fire shape, fail-open shape, lifecycle (tokio-runtime-drop, no explicit shutdown).
  - The existing per-agent startup spawn pattern in the agent-init path.

  **Test expectation:**
  - Unit test: `spawn_resolver_tick_task` returns a `JoinHandle`; aborting cancels cleanly.
  - Integration test demonstrating the **relative-rate property** (drain rate ≥ extraction rate, the AC's actual property) — F7 from architect first-pass:
    - Seed DB with 5 pending Stage-2 subjects.
    - Use a test-only 100ms interval (parameterize `interval_secs` for tests).
    - Set budget to 2 (test-only override) so each tick can resolve at most 2 entities.
    - Tick 1: drains 2 → 3 pending.
    - Between tick 1 and tick 2: simulate an extraction event adding 3 new pending subjects → 6 pending.
    - Tick 2: drains 2 → 4 pending.
    - Between tick 2 and tick 3: simulate another extraction adding 3 → 7 pending.
    - Tick 3: drains 2 → 5 pending.
    - **Assertion:** end-state pending count (5) ≤ initial pending count (5). Drain rate of 2/tick matches extraction rate of (avg) 2/tick over 3 ticks. Relative-rate property holds.
    - This fixture demonstrates the AC's actual property (relative drain) rather than just the mechanism (drain happens).

  **Verification:**
  - `grep -rn 'kg_resolver_tick' crates/mika-agent/src/` — new logs appear in expected locations.
  - Run `mika-server`, watch logs for `kg_resolver_tick.start` / `kg_resolver_tick.complete` events firing every 30 min per enabled agent.
  - `mika kg status --agent mika-arch` over multiple half-hour windows — pending count trends to 0 without restart.
  - `cargo test -p mika-agent kg::resolver_tick` — both unit + integration tests pass.

- [ ] **Unit 2: Documentation updates for the new execution context**

  **Goal:** Document the third resolver execution context and the new operator signal so future contributors and operators understand the drain model.

  **Requirements:** R3 (Signal E documentation), R4 (cost predictability), R5 (operator escape hatch reaffirmed).

  **Dependencies:** Unit 1 (documents the implemented behavior).

  **Files:**
  - Modify: `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Entity Resolver — add "Periodic tick" as the third execution context after "(1) Startup: background tokio::spawn..." and "(2) Compound hook: synchronous inline...".
  - Modify: `crates/mika-agent/CLAUDE.md` § Post-restart safety check #757 — add **Signal E**: *"Tick drain — `kg_resolver_tick.complete` events with `pending_after` trending to 0 over hourly windows. Steady-state mika-arch primary corpus should reach `pending_after == 0` within ~17–18 hours of post-restart drain. Sustained `aborted_budget = true` indicates operator should raise `MIKA_KG_BATCH_BUDGET` temporarily for accelerated drain."*
  - Modify: `mika/CLAUDE.md` § Environment Variables — `MIKA_KG_BATCH_BUDGET` description gains a note: *"Default 500 per #757 burst-defense invariant. Steady-state drain is now decoupled from restart cadence via the 30-min resolver tick (#906); raising this only needed for accelerated one-time backlog drain after deploy or migration."*

  **Approach:**
  - Edit prose; no code changes.
  - Cross-link Unit 1's log event names so operators can grep them.

  **Test expectation:** None — prose changes only.

  **Verification:** `mika/CLAUDE.md` and `crates/mika-agent/CLAUDE.md` reflect the new behavior; Signal E appears in the post-restart safety check enumeration.

## System-Wide Impact

- **Interaction graph:** The new tick joins the startup background spawn and compound-hook synchronous spawn as the third caller of `resolve_pending`. All three use the same `kg_resolutions_log UNIQUE(agent_id, subject_entity_id)` constraint as the deduplication mechanism, so race conditions between them result in one fast no-op rather than double-resolution (F5 race analysis).
- **Error propagation:** Tick failures log-and-skip per C2.3 — they do not propagate to the agent loop, do not affect message handling, do not abort the tokio runtime. Same isolation as `checkpoint_task`.
- **State lifecycle risks:**
  - Tick fires while compound-hook spawn is mid-flight: covered by `kg_resolutions_log` UNIQUE constraint.
  - Tick fires during agent shutdown: tokio cancels the task at runtime drop; in-flight `resolve_pending` either completes or is dropped mid-call. Mid-call drop leaves no orphan rows because `resolve_pending` writes inside transactions.
  - DB connection unavailable at tick fire: log-and-skip; next tick retries.
- **API surface parity:** No new public APIs. `MIKA_KG_BATCH_BUDGET` env var continues to work as the operator escape hatch.
- **Burst invariant (mika#757) preserved:** every Stage-2 batch — startup, compound-hook, OR tick — still capped at 500 calls. Throughput improvement comes from cadence, not burst size. *F1 ratification.*
- **Unchanged invariants:** Stage-1 exact-match path unchanged. Idempotency contracts unchanged. `kg_extractions` table unchanged. Compound hook behavior unchanged. Sole-writer contract for `kg_subject_resolutions` and `kg_resolutions_log` unchanged. Default budget unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Cost ramp during drain windows: many agents × 30-min ticks × backlogged corpora produces ongoing LLM spend. | Bounded by `kg_batch_budget = 500` per tick (not raised). Empirical bound: 4 KG-enabled agents × 500 calls × 48 ticks/day × ~$0.0001/call = ~$10/day at full burn. Drained-state cost ≈ $0/day (Pin 1 short-circuit). Operator can raise tick interval (future env var) or temporarily lower budget if cost ramps unexpectedly. |
| Tick fires while DB is migrating or otherwise unavailable. | log-and-skip per C2.3; next tick retries. No data loss because resolver is idempotent. |
| Multiple `resolve_pending` invocations interleave (tick + compound hook + concurrent test runner). | Resolver writes are inside transactions; `UNIQUE(agent_id, subject_entity_id)` on `kg_resolutions_log` prevents double-counting. Race surface analysis (F5): `count_pending` staleness is bounded — under-count → tick skips, next tick catches up; over-count → wasted tick, no corruption. Both safe. |
| Tick interval too aggressive for low-pending steady state, producing wasteful idle queries. | Pin 1 confirms `resolve_pending` short-circuits with one SQLite query when `pending.is_empty()`. Idle cost ≈ 1 query per 30 min per agent. Negligible. |
| Future operator runs `MIKA_KG_BATCH_BUDGET=0` expecting "disable resolver" — but with tick task spawned, the tick still fires (just resolves zero per call). | Existing `budget=0` semantics preserved per `crates/mika-agent/src/kg/entity_resolver.rs` (line 532 `// #757: respect per-batch budget before issuing the LLM call`). Tick fires, resolver short-circuits before LLM call when `budget=0`. Document explicitly in Unit 2's CLAUDE.md update. |
| Tick lifecycle outlives DB connection or agent state. | `checkpoint_task` precedent shows tokio-runtime-drop is sufficient for non-IO-holding tasks. Tick body fully bounded per iteration. No mitigation required. |

## Future Work (deferred per #757-style lazy-introduction)

- **`MIKA_KG_RESOLVER_TICK_INTERVAL_SECS` env var.** 30-min hard-coded for v1. Add when an operator scenario emerges (matches `MIKA_KG_BATCH_BUDGET`'s introduction in #757 — added in response to incident, not pre-emptively).
- **Tick disable env var (`MIKA_KG_RESOLVER_TICK_DISABLE=1`).** Useful in test environments. Add when operator scenario emerges. Today, setting `kg.enabled=false` in `identity.toml` already disables the tick per the agent's KG config check.
- **Per-tick budget refresh.** If steady-state drain proves to need more aggressive throughput than 500/30min provides AND operator runbook for env override is insufficient, consider a second tick at a shorter interval with a smaller budget (e.g., 250 every 15 min). Defer until empirical data motivates it.

## Sources & References

- Related issue: mika#906
- Closed-as-not-a-bug sibling: mika#875 (Stage-1 disconfirmation evidence)
- Companion fix: mika#874 (Stage-2 candidate-list rejection — once shipped, the resolver's `matched_llm` rate climbs and #906's drain-rate calculus improves further)
- Sibling milestone tickets: mika#876 (extraction quality), mika#877 (secondary corpora)
- Foundational invariant: mika#757 ("no silent multi-thousand-call bursts" + "leave remaining work for subsequent deferred runs")
- Code references:
  - `crates/mika-agent/src/kg/entity_resolver.rs:233-251` — `resolve_pending` with empty-pending short-circuit (Pin 1)
  - `crates/mika-agent/src/kg/entity_resolver.rs:152-156, 707-720` — `docs_root_hashes` IN-list multi-corpus iteration (Pin 2)
  - `crates/mika-agent/src/server/checkpoint.rs::spawn_dashboard_checkpoint_task` — periodic tokio task pattern (Pin 3)
  - `crates/mika-agent/src/kg/ingestion_orchestrator.rs` — compound-hook synchronous spawn (companion execution context)
- Documentation references:
  - `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Entity Resolver, § Post-restart safety check #757
  - `mika/CLAUDE.md` § Environment Variables (`MIKA_KG_BATCH_BUDGET`, Signal D)
- Institutional learnings:
  - `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` (the disconfirmation procedure that surfaced #906)
  - `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` (structural-vs-prompt-rule pattern — informs Shape 5 rejection)
- Architect first-pass review session: `48072601-2ffc-48ce-be02-9ff591f7591e` (2026-04-30, mika-arch via mika-arch-groom-ticket skill)
