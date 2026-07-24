# Plan — fix(auto_pull): stuck-ready reconciler (Phase 2)

**Ticket:** mika#1824
**Type:** fix (loop substrate, p1-important)
**Branch:** `fix/1824/auto-pull-add-stuck-ready-reconciler`
**Target file:** `crates/mika-agent/src/auto_pull.rs` (extend existing poller — no new cron)

---

## Context

`auto_pull_groomed_ticket()` (mika#1363) is a 10-min poller (`AUTO_PULL_CRON = "0 */10 * * * *"`,
wired at `server/mod.rs:1436`, dispatched at `dispatcher.rs:854`). It promotes a groomed-**not**-ready
ticket to `ready` so the webhook path dispatches it. Two design choices make it blind to a distinct
failure mode — *"ticket already has `ready` but nothing happened"* (webhook dropped, mika-dev busy at
fire time, dispatch-gate silent-accept):

1. **The `ready` filter excludes them.** `select_best_candidate` (`auto_pull.rs:93`):
   `.filter(|i| !i.labels.iter().any(|l| l.name == "ready"))`. A ticket that *has* `ready` but was
   never dispatched is filtered out — the poller cannot re-drive it.
2. **The queue-empty gate blocks the whole tick.** `auto_pull_groomed_ticket` returns early when
   `count_active_self_dev_tasks() > 0` (`auto_pull.rs:272`). While mika-dev is busy the poller does
   nothing at all.

**Founding incident (2026-07-23):** 4 re-kicks (`#1660/#1667/#1712/#1664`) produced 1 dispatch;
3 tickets kept the `ready` label but never got a callback child. No `webhook_deliveries` DLQ entries.

This ticket adds a **Phase 2 stuck-ready reconciler** to the same poller. It is the recovery net for
dispatch loss, **not** the primary fix for the dispatch-layer silent-drop classes (explicitly out of
scope — separate ticket, #1291-adjacent).

---

## Requirements

- **R1** — On each 10-min tick, run Phase 2 **after** Phase 1, in the **same** `auto_pull_groomed_ticket`
  invocation. Phase 1 behavior is unchanged.
- **R2** — Phase 1 keeps its `count_active_self_dev_tasks() > 0` early-return. Phase 2 must run **even
  when the queue is non-empty** — a stuck-ready ticket by definition has no active self_dev task *for
  itself*, and the per-ticket in-flight filter (R4) already prevents competing with in-flight work.
  → This forces a restructure: the queue-empty gate can no longer be a bare early-return at the top of
  the function.
- **R3** — Phase 2 selects candidate tickets that (a) **have** the `ready` label, (b) whose `ready`
  label was applied more than `STUCK_READY_THRESHOLD_SECS` ago (env
  `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`, default `900`), (c) have **no** in-flight (`pending`/`in_progress`)
  self_dev task referencing that issue, and (d) have **no** open PR closing the issue.
- **R4** — For each surviving stuck-ready ticket, remove-then-add the `ready` label via `gh issue edit`
  (Option A — reuse the whole webhook pipeline; no new dispatch code path). Emit `stuck_ready_reconciled`
  INFO on success.
- **R5** — Extend the existing per-issue circuit breaker (`auto_pull_stats`, threshold
  `CIRCUIT_BREAKER_THRESHOLD = 3`) to Phase 2: skip a ticket whose failure count ≥ threshold (DEBUG log);
  increment on `gh` failure, reset on success — identical to Phase 1's mechanism.
- **R6** — Emit `stuck_ready_reconcile_skipped` DEBUG per skip reason (`in_flight_self_dev`,
  `open_pr_closing`, `circuit_breaker`, `below_threshold`).
- **R7** — Idempotency of the remove→add race (AC4) needs **no new code** — the dispatch path's existing
  `has_active_callback_child` guard already de-dupes a webhook that arrives between the remove and the add.

---

## Design decisions

### D1 — Label-age source: issue timeline, not `updatedAt`

`gh issue list --json number,body,labels,updatedAt` does **not** expose *when the `ready` label was
applied*. `updatedAt` moves on any edit and is the wrong signal. AC2 requires *age since label applied*,
so Phase 2 must read the issue timeline:

```
gh api "repos/senara-solutions/mika/issues/<n>/timeline" \
  --jq '[.[] | select(.event=="labeled" and .label.name=="ready") | .created_at] | last'
```

The **last** `labeled(ready)` event's `created_at` is the authoritative apply-time (a remove→add cycle
resets it — see D3). Parse ISO 8601, compare to `now`. This is **one API call per candidate**, so the
call is placed **last** in the filter chain (D2) to minimise cost.

Add a private helper `async fn gh_ready_label_age_secs(github_token, issue_number) -> Result<Option<i64>>`
returning `None` when no `labeled(ready)` event exists (treat as not-stuck / skip). Fail-open: on API
error, log WARN and skip that ticket (do not rescue on unknown age).

### D2 — Filter ordering (cheapest → most expensive) to bound API cost

Phase 2 reuses data already fetched in the same tick:

1. **In-memory** — from the already-fetched `issues` list, keep only those **with** the `ready` label
   (free; inverse of Phase 1's filter).
2. **In-memory** — drop those in `open_pr_issue_numbers` (already fetched by
   `gh_list_open_pr_closing_issues` for Phase 1) → skip reason `open_pr_closing`.
3. **DB (cheap)** — drop those with an in-flight self_dev task for the issue (R4 new DB method) → skip
   reason `in_flight_self_dev`.
4. **DB (cheap)** — drop those with circuit-breaker failure ≥ threshold → skip reason `circuit_breaker`.
5. **GitHub API (one call each)** — for the *survivors only*, fetch `ready` label age; drop those below
   threshold → skip reason `below_threshold`.

Steps 1–4 eliminate almost every ticket before any timeline API call fires.

### D3 — Self-throttling via label-age reset (natural backoff)

Rescuing **every** match each tick is safe precisely because the remove→add cycle **resets the
`labeled(ready)` timestamp**. A ticket rescued but still not dispatched will read age ≈ 0 on the next
tick and stay ineligible for another `STUCK_READY_THRESHOLD_SECS`. This is a built-in rate limiter — no
separate cooldown state is needed. A defensive `MAX_STUCK_RESCUE_PER_TICK` cap (default `5`) bounds a
pathological tick; overflow is logged and left for the next tick.

### D4 — Per-issue circuit breaker (reuse), not a global Phase-2 cooldown

The ticket text mentions "skip Phase 2 for `CIRCUIT_BREAKER_COOLDOWN`". There is no existing time-based
cooldown mechanism; the existing breaker is the per-issue count in `auto_pull_stats`. Reusing it (skip a
ticket at failure ≥ 3, increment on `gh` error, reset on success) achieves the actual goal — a
persistently-failing ticket stops being retried — with **zero new schema/state**. This is the chosen
interpretation; documented here as a deliberate divergence from the ticket's cooldown phrasing.

### D5 — Function restructure (R2)

Rename the current single-purpose body into `phase1_promote_groomed(...) -> Option<u64>` (the existing
logic verbatim, including its own queue-empty gate) and add `phase2_reconcile_stuck_ready(...) -> usize`
(count rescued). `auto_pull_groomed_ticket` becomes the orchestrator:

```rust
pub async fn auto_pull_groomed_ticket(db, github_token, trace_id, session_id) -> Option<u64> {
    // fetch issues + open_pr set ONCE, share across both phases
    let issues = gh_list_open_issues(...).await.ok()?;              // WARN+return None on error
    let open_pr = gh_list_open_pr_closing_issues(...).await.unwrap_or_default();

    // Phase 1 — unchanged semantics (its own queue-empty gate lives inside)
    let promoted = phase1_promote_groomed(db, github_token, &issues, &open_pr, trace_id, session_id).await;

    // Phase 2 — runs regardless of queue depth
    let rescued = phase2_reconcile_stuck_ready(db, github_token, &issues, &open_pr, trace_id, session_id).await;
    debug!(rescued, "auto_pull: phase 2 stuck-ready reconciler complete");

    promoted   // return type preserved for the dispatcher log at dispatcher.rs:880
}
```

Sharing the two `gh` list calls across both phases avoids doubling GitHub API load. Note: the current
queue-empty gate is fetched *before* the issue list; moving it inside `phase1_*` means the issue list is
fetched every tick even when the queue is busy — acceptable (one `gh issue list` per 10 min) and now
**required** because Phase 2 needs that list.

### D6 — New DB method for the per-issue in-flight check (R4)

`count_active_self_dev_tasks` is agent-wide, not per-issue. Add, mirroring `has_completed_groom_for_issue`
(`db.rs:6599`):

```rust
/// True if an active (pending/in_progress) self_dev task references this issue.
pub fn has_active_self_dev_task_for_issue(&self, agent_id: &str, issue_url: &str) -> Result<bool>
```

Match on `reference_url` with a `LIKE issue_url || '%'` prefix so the `?phase=groom` suffix variant is
covered. Expose the async wrapper on `AsyncDatabase` (uses `self.agent_id()`, like
`count_active_self_dev_tasks` at `async_db.rs:508`). `issue_url` is built as
`https://github.com/senara-solutions/mika/issues/<n>`.

---

## Implementation steps

1. **Constants** (`auto_pull.rs`): add `STUCK_READY_THRESHOLD_DEFAULT_SECS: i64 = 900`,
   `STUCK_READY_THRESHOLD_ENV: &str = "MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS"`,
   `MAX_STUCK_RESCUE_PER_TICK: usize = 5`. Add a small `stuck_ready_threshold_secs()` reader
   (env parse → default on missing/invalid, WARN on invalid).
2. **`gh_remove_label`** helper (`auto_pull.rs`): mirror `gh_apply_label` (`auto_pull.rs:223`) with
   `--remove-label`. Tolerate "label not present" as success (idempotent).
3. **`gh_ready_label_age_secs`** helper (D1): `gh api .../timeline` + `--jq`, parse last
   `labeled(ready)` `created_at`, return `Option<i64>` age in seconds. Fail-open (skip on error).
4. **DB method** (D6): `Database::has_active_self_dev_task_for_issue` + `AsyncDatabase` wrapper.
5. **Refactor** (D5): extract `phase1_promote_groomed` (verbatim existing logic + its queue gate),
   rewrite `auto_pull_groomed_ticket` as the two-phase orchestrator sharing the fetched lists.
6. **`phase2_reconcile_stuck_ready`** (D2 ordering, D3 rescue loop): filter chain 1→5, emit
   `stuck_ready_reconcile_skipped` DEBUG per skip, then per survivor: circuit-breaker guard → remove →
   add → on success `record_auto_pull` + `reset_auto_pull_failure` + `stuck_ready_reconciled` INFO;
   on failure `increment_auto_pull_failure` + WARN. Cap at `MAX_STUCK_RESCUE_PER_TICK`.
7. **Selection unit-test seam:** extract the pure predicate
   `select_stuck_ready_candidates(issues, open_pr, in_flight_issue_numbers, now, ages_by_issue, threshold)
   -> Vec<u64>` so AC5 is testable with fixtures and **no network/DB** (mirrors `select_best_candidate`'s
   pure-function testability). The `phase2_*` async fn wires real `gh`/DB into this predicate.
8. **Tests** (AC5): fixture with mixed tickets — `ready+in_flight`, `ready+stuck` (age > threshold, no
   in-flight, no PR), `ready+open_pr`, `ready+fresh` (age < threshold), `not-ready+groomed`. Assert only
   `ready+stuck+no-in-flight+no-PR+age>threshold` is selected. Add threshold-reader tests
   (default/valid/invalid) and a `gh_ready_label_age_secs` timeline-parse unit test over a captured JSON
   fixture.

---

## Verification contract

- `cargo test -p mika-agent auto_pull` — all existing + new unit tests green.
- `cargo clippy -p mika-agent -- -D warnings`, `cargo fmt --check`.
- New pure predicate `select_stuck_ready_candidates` covered by the AC5 mixed-fixture test.
- No new cron, no new webhook route, no change to Phase 1's observable behavior (existing Phase 1 tests
  unchanged and passing).

### Post-deploy signals (operator, AC6)

- `grep stuck_ready_reconciled $MIKA_SPIRIT_LOG_FILE | jq 'select(.agent_id=="mika-dev")'` — expect
  **≤5/day** steady-state (transient-drop baseline). **>20/day** ⇒ Phase-1/dispatch-layer fix
  insufficient; escalate to the dispatch-layer primary fix (out of scope here).
- `grep stuck_ready_reconcile_skipped $MIKA_SPIRIT_LOG_FILE` — skip-reason distribution for tuning
  the threshold.

---

## Definition of Done

- Phase 2 runs after Phase 1 on every tick, independent of queue depth (R1, R2).
- Stuck-ready selection matches R3 with the D2 cost-bounded ordering.
- Remove→add rescue emits `stuck_ready_reconciled`; skips emit `stuck_ready_reconcile_skipped` (R4, R6).
- Circuit breaker extended to Phase 2 via the existing per-issue counter (R5, D4).
- `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` documented in `mika/CLAUDE.md` env section.
- AC5 unit test passes; clippy/fmt clean; existing Phase 1 tests unchanged.

---

## Acceptance criteria

- **AC1** — `auto_pull_groomed_ticket` runs Phase 2 after Phase 1 on each cron fire. Skips Phase 2 with
  DEBUG log when circuit breaker open.
- **AC2** — Phase 2 identifies tickets with `ready` label + age > `STUCK_READY_THRESHOLD_SECS`
  (configurable env `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`, default 900) + no in-flight self_dev
  task + no open PR closing.
- **AC3** — Per stuck-ready ticket, Phase 2 removes-then-adds `ready` via `gh issue edit`, emits
  `stuck_ready_reconciled` INFO log.
- **AC4** — Ticket already dispatched between the remove and the add (webhook race) is idempotent: the
  second re-add doesn't create a duplicate dispatch (existing `has_active_callback_child` guard on the
  dispatch path catches this — no new work needed).
- **AC5** — Unit test in `auto_pull.rs`: given a fixture with mixed tickets (some ready+in-flight, some
  ready+stuck, some not ready), assert only ready+stuck+no-in-flight are selected.
- **AC6** — Post-deploy: run for one 24h cycle; grep `stuck_ready_reconciled` events; expect ≤5 per day
  (baseline noise from transient drops); if >20/day, indicates fragility 1 fix insufficient and dispatch
  layer needs primary fix.

---

## Out of scope

- Fixing the dispatch-layer silent-drop classes (4xx non-429 → DLQ; route-unresolved → DLQ; webhook
  agent-side dispatch-gate silent-accept observability). Separate ticket, #1291-adjacent.
- Direct HTTP dispatch bypass (Option B) — Option A chosen; Option B would duplicate
  retry/circuit-breaker/dedup that the webhook path already owns.
- A cron for DLQ pending→dead transitions (already covered by `dlq.rs::run_dlq_worker`).
- PR-side reconciler for stuck `ready_for_review` PRs (symptom of fragility 3).

---

## Risks

- **R-timeline-cost** — one `gh api timeline` call per surviving candidate. Mitigated by D2 ordering
  (survivors are rare) and the D3 label-age reset (rescued tickets exit the survivor set for a full
  threshold window).
- **R-thundering-herd** — many simultaneous rescues re-firing webhooks. Mitigated by `MAX_STUCK_RESCUE_PER_TICK`
  and the dispatch gate serialising actual dispatches; non-dispatched re-kicks self-throttle via D3.
- **R-refactor-regression** — extracting `phase1_promote_groomed` must preserve Phase 1 semantics
  exactly. Mitigated by keeping the existing Phase 1 unit tests unchanged and green.

## References

- mika#1363 (`auto_pull_groomed_ticket` foundation), mika#1517 (open-PR filter shape),
  mika#1710 (dispatch circuit breaker). Founding incident: batch re-kick 2026-07-23.
