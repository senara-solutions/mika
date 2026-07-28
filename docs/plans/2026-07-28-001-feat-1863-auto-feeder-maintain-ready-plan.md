# Plan — feat(task-engine): auto-feeder — maintain ready-count ≥ N via groomed-backlog promotion

**Ticket:** mika#1863 (P0 loop-productivity fix A of 3)
**Type:** feat (loop substrate, p1-important, agent-core)
**Branch:** `feat/1863/task-engine-auto-feeder-maintain-ready`
**Target file:** `crates/mika-agent/src/auto_pull.rs` (extend the existing two-phase orchestrator — **no new cron**)

---

## Context

Overnight 2026-07-27→28 the autonomous loop idled **11 hours** despite `auto_pull_groomed_ticket()`
ticking correctly every 10 min. Root cause: the `ready` pool drained to zero-pullable and nothing
re-fed it. The queue *held* two `ready`-labelled tickets (#1682 with an open PR, #1646 in-flight) but
both were predicate-skipped — so the raw `ready` count was non-zero while the **pullable** count was
zero. Manual re-labelling is not durable; the structural fix is a proactive feeder that keeps the
pullable-ready pool topped up to a threshold.

### Where this fits in the existing module

`crates/mika-agent/src/auto_pull.rs` already owns the `ready`-label maintenance surface as a two-phase
orchestrator, dispatched every 10 min (`AUTO_PULL_CRON = "0 */10 * * * *"`, `server/mod.rs:88`; wired
`server/mod.rs:1435`; dispatched `dispatcher.rs:872`, mika-dev only, gated by `MIKA_DEV_AUTO_PULL`):

- **Phase 1** (`phase1_promote_groomed`) — queue-idle → promote the single best groomed-not-ready
  ticket to `ready`. Demand-driven, reactive, fires only when `count_active_self_dev_tasks() == 0`.
- **Phase 2** (`phase2_reconcile_stuck_ready`, mika#1824) — remove→add re-drive tickets that carry
  `ready` but were never dispatched. Runs regardless of queue depth.

The orchestrator fetches the open-issue list and the open-PR closing-issue set **once** and shares both
across phases (mika#1824 D5). The feeder is a natural **Phase 0** in the same orchestrator: it needs the
same two fetches, the same `is_groomed`/`gh_apply_label`/circuit-breaker/`has_active_self_dev_task_for_issue`
machinery, and it must run *before* Phase 1 in the same tick (AC2).

### Deliberate divergence from the ticket's stated location

The ticket says "add periodic task `auto_feeder` to `crates/mika-agent/src/task_engine/`". The actual
sibling `auto_pull_groomed_ticket` lives in `crates/mika-agent/src/auto_pull.rs`, not `task_engine/`.
The binding intent — *"alongside existing `auto_pull_groomed_ticket`"* — is honoured by folding the
feeder into that orchestrator. Renaming the module (e.g. `auto_pull.rs` → `ready_pool.rs`) to reflect
its now-three-phase scope is a tempting nicety but **out of scope** (would churn imports + test paths for
zero behavioural gain).

---

## Requirements

- **R1 — Phase 0, before Phase 1, same tick.** Add `phase0_feed_ready_pool(...)` to
  `auto_pull_groomed_ticket`, invoked **before** `phase1_promote_groomed`, sharing the already-fetched
  `issues` and `open_pr_issue_numbers`. This structurally guarantees AC2's "feeder promotes → puller
  picks up same tick" — a separate recurring task at the same cron gives **no** intra-tick ordering
  guarantee (the engine fires due tasks from a heap in nondeterministic order) and would double the
  `gh issue list` API load. Phase 1 and Phase 2 behaviour is unchanged.
- **R2 — Config `MIKA_AUTO_FEEDER_MIN_READY`** (AC1). Default `3`; clamp to `[1, 10]`; the literal value
  `0` **disables** the feeder (Phase 0 returns early). Missing/invalid → default `3` (WARN on invalid).
  Parsed by a pure `parse_min_ready(raw: Option<&str>) -> u32` (unit-testable, no env mutation), mirroring
  `parse_stuck_ready_threshold`.
- **R3 — Pullable-ready count is the threshold signal**, not the raw `ready` count. A ticket counts
  toward the pool iff it (a) has `ready`, (b) has **no** open PR closing it, (c) has **no** in-flight
  self_dev task, (d) is **not** labelled `blocked` or `operator-review`. If pullable-count ≥ `min_ready`
  → no-op (`auto_feeder.skip`). This is a **deliberate divergence from AC3's literal** `gh issue list
  --label ready | jq length` — the founding incident (#1682 open-PR + #1646 in-flight, both
  predicate-skipped) proves raw-count is the wrong signal for the stated goal ("never idle on empty
  pullable queue"). See D2.
- **R4 — Backlog = groomed-not-ready, dispatchable.** Candidates are open issues that (a) do **not** have
  `ready`, (b) pass `is_groomed(body)` (full canonical callout — Branch + Plan + second-pass GROOMED),
  (c) have **no** open PR, (d) have **no** in-flight self_dev task, (e) are **not** `blocked`/`operator-review`.
  Sorted by `feeder_rank` (R6) then oldest-first. Working set capped at 50 (AC4). The `is_groomed`
  predicate — **not** the loose `"Plan: docs/plans/"` substring from AC4 — is mandatory: the dispatch
  gate (#919) rejects any promoted ticket lacking the full callout with `dispatch_no_grooming_marker`,
  which would manufacture exactly the stuck-ready churn Phase 2 exists to clean up. See D3.
- **R5 — Promote up to `min_ready − pullable_count`** top candidates (AC5), each via `gh_apply_label(…,
  "ready")`, guarded by the existing per-issue circuit breaker (`auto_pull_stats`, threshold 3): skip at
  failure ≥ 3, increment on apply failure, reset + `record_auto_pull` on success — identical to Phase 1.
- **R6 — `feeder_rank(labels) -> u8`** keyed on the **real** label taxonomy (`.github/labels.yml`):
  `p0-critical`=5, `p1-important`=4, `agent-core`=3, `p2-normal`=2, `p3-nice-to-have`=1, none=0. Rank is
  the **max** per-label rank (so `#1863`, which is both `p1-important` and `agent-core`, ranks 4). This
  realises AC4's "p1-important > agent-core > p2/p3" and sidesteps the latent bug in `priority_rank`
  (which matches bare `"p1"` and returns 0 for every real issue — out of scope to fix here).
- **R7 — Observability (AC6).** Emit `auto_feeder`-tool-name audit events: `auto_feeder_promoted` per
  apply (issue, rank, reason), `auto_feeder_skip` when pullable-count ≥ threshold, `auto_feeder_no_backlog`
  when the pool is under threshold but zero dispatchable backlog exists (true starvation signal). Paired
  INFO logs for grep. `tool_name = "auto_feeder"` matches AC9's verify query.
- **R8 — Fail-open (AC7).** The feeder consumes the shared `gh issue list` / `gh pr list` fetches, which
  already fail-open in the orchestrator (WARN + `return None` / empty set). No new GraphQL is introduced,
  so AC7's "fail-open on GraphQL errors" is satisfied by construction. A `gh_apply_label` failure warns +
  increments the circuit breaker and moves to the next candidate; it never propagates or crashes the tick.

---

## Design decisions

### D1 — Phase 0 in the orchestrator, not a separate recurring task

Chosen over a sibling cron task for three reasons: (1) AC2 ordering is structural, not scheduling-luck;
(2) the feeder reuses the two shared `gh` fetches — zero added GitHub API load per tick; (3) it reuses
`is_groomed`, `gh_apply_label`, the circuit breaker, and `has_active_self_dev_task_for_issue` verbatim.
Mirrors mika#1824's decision to fold Phase 2 into the same orchestrator rather than spawn a new cron.

Orchestrator shape after this change:

```rust
pub async fn auto_pull_groomed_ticket(db, github_token, trace_id, session_id) -> Option<u64> {
    let issues = gh_list_open_issues(...).await?;               // WARN + return None on error
    let open_pr = gh_list_open_pr_closing_issues(...).await.unwrap_or_default();

    // Phase 0 — feeder: top the pullable-ready pool up to MIN_READY (new). Runs first.
    let fed = phase0_feed_ready_pool(db, github_token, &issues, &open_pr, trace_id, session_id).await;
    debug!(fed, "auto_pull: phase 0 feeder complete");

    // Phase 1 — puller: queue-idle single promotion (unchanged).
    let promoted = phase1_promote_groomed(db, github_token, &issues, &open_pr, trace_id, session_id).await;

    // Phase 2 — stuck-ready reconciler (unchanged).
    let rescued = phase2_reconcile_stuck_ready(db, github_token, &issues, &open_pr).await;
    debug!(rescued, "auto_pull: phase 2 stuck-ready reconciler complete");

    promoted   // return type preserved for the dispatcher log at dispatcher.rs:880
}
```

**Phase-interaction note (benign over-promotion).** After Phase 0 tops the pool to N, Phase 1 (if the
queue is idle) may still promote one *additional* groomed-not-ready ticket — its filter is `!ready`, and
the feeder just consumed the top candidates. Result: at most N+1 ready. This is harmless: the webhook
dispatch drains the pool, and Phase 1's intent (kick a dispatch when idle) is complementary to the
feeder's (hold a buffer). No coupling change to Phase 1 is made.

### D2 — Pullable-count, not raw `ready`-count (divergence from AC3)

AC3 specifies `gh issue list --label ready | jq length`. The founding incident proves that signal is
wrong: raw-count was ≥ 1 while pullable-count was 0, and the loop idled 11 h. The feeder counts a ready
ticket toward the pool only if it is actually dispatchable (no open PR, no in-flight task, not
`blocked`/`operator-review`). Computed **in-memory** from the shared `issues` + `open_pr` + a small set
of `has_active_self_dev_task_for_issue` DB probes — no extra GitHub call. This is the single most
important correctness decision in the plan; flagged explicitly for architect review.

### D3 — `is_groomed()` (full callout), not the `"Plan: docs/plans/"` substring (divergence from AC4)

AC4's grooming signal ("body contains `Plan: docs/plans/`") is weaker than the dispatch gate's
requirement. `validate_dispatch_readiness()` check (5) (#919) rejects a dev-pilot dispatch unless the
body carries **all three** canonical callouts (`> - **Branch:**`, `docs/plans/`, a `second-pass (GROOMED)`
marker). Promoting a ticket that has only `Plan:` would bounce at the gate as `dispatch_no_grooming_marker`
and leave a stuck-ready ticket — the exact failure class mika#1824 fights. The feeder therefore reuses
`is_groomed()` so every promoted ticket is genuinely dispatchable.

### D4 — Pure selection seam for testability

Extract a pure predicate mirroring `select_best_candidate` / `select_stuck_ready_candidates`:

```rust
/// Given the shared issue list + resolved skip-sets, return the issue numbers to promote,
/// highest feeder_rank first (oldest-first tiebreak), capped at `slots` (= min_ready − pullable).
fn select_feeder_candidates(
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    in_flight_issue_numbers: &HashSet<u64>,
    slots: usize,
) -> Vec<u64>
```

Plus a pure `count_pullable_ready(issues, open_pr, in_flight) -> usize`. Both are network/DB-free and
carry the AC8 unit coverage. The async `phase0_*` wrapper resolves the `in_flight` set via
`has_active_self_dev_task_for_issue` (one cheap DB probe per ready-or-candidate ticket, bounded by the
50-cap) and wires the predicate to real `gh`/DB — same split as Phase 2.

### D5 — Circuit breaker + audit reuse

Reuse `auto_pull_stats` (per-issue, threshold 3) for feeder applies — a persistently-failing issue stops
being re-promoted. Audit via the existing `log_audit_event(session_id, "auto_feeder", "<target_key>", …,
trace_id)` signature. `WORKING_SET_CAP: usize = 50` and `AUTO_FEEDER_MIN_READY_ENV`/`_DEFAULT`/`_MIN`/`_MAX`
consts added next to the Phase 2 consts.

---

## Implementation steps

1. **Consts** (`auto_pull.rs`): `AUTO_FEEDER_MIN_READY_ENV = "MIKA_AUTO_FEEDER_MIN_READY"`,
   `AUTO_FEEDER_MIN_READY_DEFAULT: u32 = 3`, `AUTO_FEEDER_MIN_READY_MIN: u32 = 1`,
   `AUTO_FEEDER_MIN_READY_MAX: u32 = 10`, `FEEDER_WORKING_SET_CAP: usize = 50`.
2. **`parse_min_ready(raw: Option<&str>) -> u32`** + `auto_feeder_min_ready()` env reader (D2/R2). `0` →
   `0` (disable sentinel, preserved through clamp); `1..=10` → as-is; `>10` → `10`; missing/invalid →
   `3` (WARN on invalid). Unit-tested without env mutation.
3. **`feeder_rank(labels: &[IssueLabel]) -> u8`** (R6) — real-label max-rank.
4. **`count_pullable_ready(issues, open_pr, in_flight) -> usize`** and
   **`select_feeder_candidates(issues, open_pr, in_flight, slots) -> Vec<u64>`** (D4) — pure predicates.
   Candidate filter chain: `!ready` → `is_groomed` → `!open_pr` → `!in_flight` → `!blocked` →
   `!operator-review`; sort `feeder_rank` DESC then `updated_at` ASC (oldest-first); take `slots`.
5. **`phase0_feed_ready_pool(db, github_token, issues, open_pr, trace_id, session_id) -> usize`** (async
   wrapper): read `min_ready`; if `0` → return 0 (disabled). Build the `in_flight` set via
   `has_active_self_dev_task_for_issue` over the union of ready + candidate tickets (bounded by cap).
   Compute `pullable = count_pullable_ready(...)`. If `pullable >= min_ready` → emit `auto_feeder_skip`
   audit + DEBUG, return 0. Compute `slots = min_ready - pullable`; `candidates =
   select_feeder_candidates(..., slots)`. If empty → emit `auto_feeder_no_backlog` audit + INFO, return 0.
   For each candidate: circuit-breaker guard → `gh_apply_label(…, "ready")` → on success
   `record_auto_pull` + `reset_auto_pull_failure` + `auto_feeder_promoted` audit + INFO; on failure
   `increment_auto_pull_failure` + WARN, continue. Return promoted count.
6. **Wire Phase 0** into `auto_pull_groomed_ticket` before Phase 1 (D1).
7. **Tests** (AC8, all pure — no network/DB, mirroring the existing 39-test pattern with `make_issue`):
   - `parse_min_ready`: default-on-missing, `0`-disables, clamp-min (`1`), clamp-max (`>10`→`10`),
     invalid-falls-back, valid-passthrough.
   - `feeder_rank`: p1-important=4, agent-core=3, `#1863`-shape (p1+agent-core)=4, p2=2, none=0.
   - `count_pullable_ready`: excludes open-PR, in-flight, blocked, operator-review; counts plain ready.
   - `select_feeder_candidates`: filters ungroomed/ready/open-PR/in-flight/blocked; rank + oldest-first
     ordering; respects `slots` cap and `FEEDER_WORKING_SET_CAP`.
   - **AC8 end-to-end-at-predicate-level**: 0 pullable + 5 groomed backlog + `min_ready=3` →
     `select_feeder_candidates(..., slots=3)` returns exactly the top 3 by rank. (The gh/DB async wrapper
     is not unit-testable in-process — same boundary as `auto_pull`'s existing tests; covered by AC9
     post-deploy verify.)
8. **Docs**: add `MIKA_AUTO_FEEDER_MIN_READY` to `mika/CLAUDE.md` env section (new "auto-feeder" bullet
   next to the auto-pull stuck-ready reconciler entry), documenting default/clamp/`0`-disables, the
   pullable-count semantics (D2), and the three audit events.

---

## Verification contract

- `cargo test -p mika-agent auto_pull` — existing 39 tests unchanged + new feeder unit tests green.
- `cargo clippy -p mika-agent -- -D warnings`, `cargo fmt --check`.
- New pure predicates (`parse_min_ready`, `feeder_rank`, `count_pullable_ready`,
  `select_feeder_candidates`) fully covered; Phase 1 / Phase 2 tests untouched and passing.
- No new cron, no new webhook route, no schema change, no `skills/bundled/self-dev/*` prompt change
  (keeps the ticket out of DECISION-CORE per its own escape-hatch clause).

### Post-deploy signals (operator, AC9)

- **Feeder running:** `sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM audit_events WHERE tool_name =
  'auto_feeder' AND created_at > datetime('now','-24 hours');"` → > 0.
- **Pool never starves:** `gh issue list --repo senara-solutions/mika --state open --label ready --json
  number | jq length` sampled over 24 h → ≥ 3 (never dips below threshold under available backlog).
- **Starvation visibility:** `grep auto_feeder $MIKA_SPIRIT_LOG_FILE | jq 'select(.event ==
  "auto_feeder_no_backlog")'` — sustained hits mean the *grooming* pipeline (not the feeder) is the
  bottleneck; the feeder is correctly signalling real backlog exhaustion.

---

## Definition of Done

- Phase 0 runs before Phase 1 on every tick, sharing the two `gh` fetches (R1, D1).
- `MIKA_AUTO_FEEDER_MIN_READY` parsed with default 3 / clamp [1,10] / `0`-disables (R2); documented in
  `mika/CLAUDE.md`.
- Pullable-ready count (not raw) drives the threshold; promotion tops the pool to `min_ready` from the
  groomed-dispatchable backlog by `feeder_rank` then oldest-first (R3–R6).
- Skip-predicates enforced: open PR, in-flight task, `blocked`, `operator-review` (R3/R4).
- Three `auto_feeder` audit events emitted (R7); fail-open preserved (R8).
- Circuit breaker reused; no promotion of un-dispatchable (non-`is_groomed`) tickets (D3, D5).
- Feeder unit tests pass; clippy/fmt clean; Phase 1/2 tests unchanged.

---

## Acceptance criteria

- **AC1** — `MIKA_AUTO_FEEDER_MIN_READY` config env var: default `3`, min `1`, max `10`; `0` disables the
  feature.
- **AC2** — Periodic execution at the same 10-min cadence as `auto_pull_groomed_ticket`, aligned so the
  feeder runs **before** the puller (feeder promotes → puller picks up same tick). Realised structurally
  as Phase 0 of the shared orchestrator.
- **AC3** — Query the current ready-count; if ≥ `MIN_READY` threshold → no-op, return early. *(Realised
  as pullable-ready count per D2 — a deliberate divergence justified by the founding incident.)*
- **AC4** — Query the groomed-not-ready backlog: open, groomed (full callout per D3), no `ready` label;
  sorted by priority (`p1-important` > `agent-core` > p2/p3 > age-DESC); working set capped at 50.
- **AC5** — Promote up to `MIN_READY − current_pullable_count` tickets by applying `ready` to the top-N
  by sort. Skip tickets with an open closing PR, an active in-flight self_dev task, or a `blocked` /
  `operator-review` label.
- **AC6** — Emit `auto_feeder.promoted` per apply (`issue_number`, `priority`, `reason`),
  `auto_feeder.skip` when threshold already met, `auto_feeder.no_backlog` when zero dispatchable backlog
  is available. (Audit `tool_name = "auto_feeder"`; `target_key` = `auto_feeder_promoted` /
  `auto_feeder_skip` / `auto_feeder_no_backlog`.)
- **AC7** — Fail-open on GitHub errors: the shared `gh` fetches WARN-and-skip (no new GraphQL introduced),
  apply failures WARN + increment the circuit breaker; the engine tick never crashes. Next tick retries.
- **AC8** — Unit tests: promotion count = `min(threshold − current, backlog_size)`; priority-sort
  correctness (`p1-important` > `agent-core`); skip-predicates (open PR, in-flight, blocked); end-to-end
  at predicate level (0 pullable + 5 groomed backlog → 3 promoted).
- **AC9** — Post-deploy 24 h verify: `auto_feeder` audit rows present (> 0); sampled ready-count stays
  ≥ 3 under available backlog.

---

## Out of scope

- Cross-repo auto-feeder (cpp equivalent lands after cpp#79 — separate ticket).
- Priority-sort sophistication beyond the label-tier + age key (graph-aware dependency sort = follow-up).
- Dashboard UI for feeder observability (v1 = audit_events + logs only).
- Fixing `priority_rank`'s bare-`"p1"` mismatch bug in Phase 1 (latent, pre-existing; feeder uses its own
  `feeder_rank`).
- Renaming `auto_pull.rs` → `ready_pool.rs` to reflect the three-phase scope.
- Any `skills/bundled/self-dev/*` prompt change — deliberately avoided to keep the ticket out of
  DECISION-CORE (per the ticket's own escape-hatch clause).

---

## Risks

- **R-over-promotion** — Phase 1 may add one ready ticket beyond the feeder's N (D1 note). Benign; the
  webhook drains the pool. No mitigation needed.
- **R-in-flight-probe-cost** — one `has_active_self_dev_task_for_issue` DB probe per ready/candidate
  ticket. Bounded by the 50-cap and the small open-issue count; the probe is a cheap indexed lookup
  (mika#1824 D6). Acceptable at one tick / 10 min.
- **R-divergence-from-AC** — D2 (pullable-count) and D3 (`is_groomed`) diverge from AC3/AC4's literal
  text. Both are justified by the founding incident and the dispatch gate; surfaced explicitly for
  architect review. If the architect prefers strict AC-literalism, the fallback is raw-count + substring
  — but that reintroduces the exact idle-on-non-empty-but-unpullable failure this ticket exists to fix.

---

## References

- mika#1363 (`auto_pull_groomed_ticket` Phase 1 foundation), mika#1824 (Phase 2 stuck-ready reconciler —
  the direct structural precedent for folding a new phase into the shared orchestrator), mika#1517
  (open-PR closing-issue filter), mika#919 (dispatch-readiness grooming-marker gate).
- Founding incident: 2026-07-28 06:46 UTC, 11 h loop-idle. Sibling P0s: mika#1862 (`.iterate/` gitignore),
  rupture D (TBD). Precondition-blocking: cpp#79.
- Orchestrator memory `feedback_always_busy_requires_feeding_queue` (2026-07-28) — this ticket is its
  structural fix.
