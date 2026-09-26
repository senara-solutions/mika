# Plan — dedup `check_suite.completed(success)` events to mika-qa (mika#1869)

**Ticket:** mika issue#1869 — `fix(webhook): dedup check_suite.completed events to mika-qa`
**Labels:** bug, p0-critical, agent-core, ready
**Type:** fix (P0 substrate — loop-productivity, "Breaks the loop" tier)

## Problem statement

A single push to a PR branch triggers up to 8 GitHub workflows (Check, validate,
Dashboard, Docs Site, Docs Sync, Byte Slice Lint, Loop Select Lint, Pipeline
Artifacts). Each workflow emits its own `check_suite.completed(success)` webhook.
The gateway routes each as a separate `[GitHub] Check suite success on {repo}
(branch: {branch})` message to mika-qa. Each message is an independent unit of
work on mika-qa's mailbox → concurrent load exceeds capacity → `429 agent busy —
message rejected` (41 `rate_limit_trip` audit rows in one hour on 2026-07-28) →
verdicts stall (0 since 09:07 UTC) → 0 merges since 12:00 UTC.

Compounding factor (out of scope for this ticket, tracked at cpp#98): op-proxy
un-drafting PRs to bridge cpp#98 re-triggers full CI runs, re-firing the whole
8-workflow cascade.

The events are **semantically redundant**: `check_suite.completed(success)` is
scoped to ONE workflow (documented invariant in
`ci_success_handler.rs` module header). The handler already re-aggregates all
required checks via `run_gh_checks` on every invocation, so N of the N events for
the same `(repo, branch, head_sha)` do identical work and produce at most one
state transition. N−1 of them are pure waste that costs a mailbox slot.

## Current behavior (grounding)

Message path for a check_suite success webhook (`crates/mika-agent/src/server/handlers.rs:897`):

1. Gateway delivers `[GitHub] Check suite success on {repo} (branch: {branch})`
   to mika-qa as a `github`-channel `MessageRequest`.
2. `handle_message` runs the structural handler chain. `ci_success_handler::try_handle_ci_success`
   (`handlers.rs:900`) self-selects on the event text.
3. Inside `try_handle_ci_success` (`ci_success_handler.rs:89`):
   - `parse_check_suite_success` extracts `(repo, branch)` — **head_sha is NOT in the webhook text**.
   - `find_open_pr(repo, branch, token)` (`ci_success_handler.rs:523`) does a
     `gh pr list --head {branch} --state open --json number,headRefOid` and returns
     `PrInfo { number, head_sha }` — **this is the first point head_sha is known**.
   - `find_pass_verdict` → stale-SHA gate → `run_gh_checks` (60s-timeout aggregation)
     → `is_behind_main` preflight → merge/gate/notify.
4. The message then proceeds to the mika-qa turn regardless of the handler's
   `Passthrough`/`Handled` verdict.

Every one of the N events walks this full path independently. Nothing deduplicates
across the burst.

Relevant existing infrastructure we will reuse:
- `dashmap = "6"` is already a workspace dependency (`crates/mika-agent/Cargo.toml:65`).
- `AsyncDatabase::count_recent_audit_events_for_target(tool_name, target_key, since)`
  (`async_db.rs:432`) — exact-match count of audit rows newer than an ISO-8601
  cutoff. Already used by the mika#1563 PR-keyed circuit breaker. Reused verbatim for AC2.
- `AsyncDatabase::log_audit_event(session_id, tool_name, target_key, before, after, reasoning, trace_id)`
  (`async_db.rs:1459`) — the handler already writes `ci_success_handler_human_gate_required`
  and `ci_success_merge` rows (`ci_success_handler.rs:328,425`).

## Design decision — where the dedup key lives

The issue's AC1 signature is `try_dedup_check_suite(repo, branch, head_sha, window) -> bool`,
requiring `head_sha`. But **head_sha is not available in the webhook text** — it
only becomes known after `find_open_pr` does a `gh pr list`. This forces an
explicit placement decision:

| Option | Site | Key | Short-circuits | Cost avoided |
|--------|------|-----|----------------|--------------|
| A (chosen) | Inside `try_handle_ci_success`, immediately after `find_open_pr` | `(repo, branch, head_sha)` — precise | The expensive tail: `find_pass_verdict`, `run_gh_checks` (60s), `is_behind_main`, merge, notify-send | ~3+ gh calls + merge attempt per dup |
| B | `handlers.rs` message entry / `webhook_queue::correlate` | `(repo, branch)` only — no head_sha | The full handler + turn | everything, but risks false-dedup of a genuine distinct push to the same branch within the window |

**Decision: implement Option A as the primary AC1 placement.** Rationale:
- It honors the issue's `(repo, branch, head_sha)` signature exactly — distinct
  pushes advance head_sha and are never falsely deduped (blast-radius invariant
  in the ticket).
- It eliminates the costly, rate-limit-adjacent work (the gh-call cascade + merge
  path) for every duplicate, which is what actually saturates on the burst.
- One cheap `gh pr list` per event remains (to obtain head_sha), but that is a
  single, fast, unauthenticated-parallel-safe call — not the 429 source.

Option B's `(repo, branch)`-only key is rejected as the *primary* mechanism
because it cannot distinguish a redundant re-fire from a legitimate second push,
violating the ticket's "distinct pushes are NOT deduped" invariant. If empirical
post-deploy data (AC4) shows the surviving one-gh-call-per-event still floods the
mailbox turn slots, a follow-up may add a coarse `(repo, branch)` pre-gate at
`handlers.rs` — explicitly deferred, not built speculatively (YAGNI).

AC2 (audit-events early-return) provides the crash-durable, cross-restart layer
that the in-memory map cannot: after a process restart the DashMap is empty, but
the audit trail persists, so a re-fired event within the window still dedups.

## Requirements

### R1 — In-memory dedup module (AC1)
New file `crates/mika-agent/src/server/check_suite_dedup.rs`.

- Public API:
  ```rust
  /// Returns `true` if a check_suite success for this exact (repo, branch, head_sha)
  /// was already seen within `window`. Records the observation as a side effect
  /// (first caller for a key returns `false` and registers it).
  pub fn try_dedup_check_suite(
      repo: &str,
      branch: &str,
      head_sha: &str,
      window: Duration,
  ) -> bool
  ```
- Backing store: a process-global `LazyLock<DashMap<String, Instant>>` (key =
  `format!("{repo}:{branch}:{head_sha}")`, value = monotonic `Instant` of first
  observation). `dashmap` is already a dependency; `Instant` (monotonic) is used
  instead of `SystemTime` to be immune to wall-clock skew and to sidestep the
  session's `Date::now`/`SystemTime::now` constraints being irrelevant here (this
  is runtime code, not a workflow script).
- Semantics:
  - On call: look up key. If present AND `now - stored < window` → return `true`
    (duplicate). Do **not** advance the timestamp (fixed window from first sighting,
    matching the ticket's "60s from first workflow completion").
  - Else insert/overwrite with `now`, return `false`.
- Bounded growth / TTL eviction: capacity ~1000 entries, 10-min entry TTL. On
  insert, if `len() >= CAP`, sweep and drop entries older than the TTL (cheap
  amortized eviction; no background task). Document the CAP and TTL as module
  constants (`DEDUP_CAP = 1000`, `ENTRY_TTL = Duration::from_secs(600)`).
- Default window constant: `DEDUP_WINDOW = Duration::from_secs(60)` (ticket spec).
- Thread-safety: `DashMap` gives interior concurrency; no outer `Mutex` needed.
- Register the module in `crates/mika-agent/src/server/mod.rs`.

### R2 — Wire AC1 into the handler (AC1 placement)
In `try_handle_ci_success` (`ci_success_handler.rs`), immediately after the
`find_open_pr` success arm yields `pr` (head_sha now known) and **before**
`find_pass_verdict`:

- Call `check_suite_dedup::try_dedup_check_suite(&event.repo, &event.branch, &pr.head_sha, DEDUP_WINDOW)`.
- If `true`: emit an `info!` structured log `ci_success_dedup.skip` with fields
  `repo`, `branch`, `head_sha`, `pr_number` (the ticket's AC4 grep target), and
  `return VerdictAction::Passthrough { enrichment: None }`.
- If `false`: proceed to the existing path unchanged.

Rationale for placing it after `find_open_pr` rather than before: we need
head_sha for the precise key, and `find_open_pr` returning `None` (post-merge
webhook) is already a Passthrough — no dedup needed there.

### R3 — Handler audit-events early-return (AC2, defense-in-depth)
Two coordinated changes in `try_handle_ci_success`:

1. **Write a dedup marker on every non-trivial invocation.** After the in-memory
   dedup passes (R2 returned `false`) and we have `(repo, pr.number, pr.head_sha)`,
   log an audit event that records "we acted on this state":
   - `tool_name = "ci_success_handler_processed"`
   - `target_key = format!("pr:{}#{}@{}", event.repo, pr.number, pr.head_sha)`
     (head_sha embedded in the key so the exact-match count query is head_sha-precise
     without needing a `LIKE` on `after_value`).
   - `reasoning = "trigger=check_suite_success dedup_marker"`, `trace_id = Some(trace_id)`.
   - Write it once, early (right after the in-memory gate), so concurrent siblings
     that cleared the empty DashMap on a cold start still find the row.
2. **Early-return check before the marker write.** Before writing the marker, query:
   ```rust
   let since = /* now - 60s, ISO-8601 UTC via crate::timestamp helpers */;
   let seen = db.count_recent_audit_events_for_target(
       "ci_success_handler_processed",
       &format!("pr:{}#{}@{}", event.repo, pr.number, pr.head_sha),
       &since,
   ).await;
   ```
   - If `Ok(n)` with `n >= 1` → `info!("ci_success_handler.dedup_skip", ...)` and
     `return VerdictAction::Passthrough { enrichment: None }`.
   - If `Ok(0)` → write the marker (step 1) and proceed.
   - If `Err(_)` → fail-open (log `warn!`, proceed) — never let an audit read block
     a legitimate merge (matches the handler's existing fail-open posture on
     `is_behind_main`).

Ordering within the handler: `find_open_pr` → **R2 in-memory dedup** →
**R3 audit early-return** → **R3 marker write** → existing `find_pass_verdict`…
The in-memory gate is checked first (cheapest); the audit gate catches the
post-restart / cross-process gap.

Note: `count_recent_audit_events_for_target` counts within the caller's `agent_id`
scope. Since check_suite success events for a given PR route to the same agent
(mika-qa), the scope is correct. Document this assumption in a code comment.

### R4 — Tests (AC3, AC5)
Inline `#[cfg(test)] mod tests` in `check_suite_dedup.rs` and additions to
`ci_success_handler.rs` tests:

- **Unit (dedup module):**
  - First call for a key returns `false`; immediate second call for the same key
    returns `true`.
  - Different head_sha for same `(repo, branch)` returns `false` (distinct push
    not deduped) — directly encodes the ticket's core invariant.
  - Window expiry: a call with a tiny window (e.g. `Duration::from_millis(0)` or a
    key pre-inserted with a back-dated `Instant`) returns `false` after the window
    elapses. Use an injectable/`pub(crate)` helper that accepts an explicit `now`
    or pre-seeds the map to avoid real sleeps.
  - Concurrency: spawn 5 threads calling `try_dedup_check_suite` with the same key;
    assert exactly one returns `false` (the AC3 "5 concurrent, only first" case).
    Use `std::thread` + an `AtomicUsize` counter of `false` returns.
  - Eviction: insert `> DEDUP_CAP` distinct keys; assert `len()` stays bounded and
    stale entries are dropped.
- **Handler-level (AC2 early-return):** unit test that pre-seeds an
  `ci_success_handler_processed` audit row for `pr:{repo}#{n}@{sha}` via a test
  `AsyncDatabase`, then asserts the audit-count path would short-circuit. If a
  full `try_handle_ci_success` invocation is impractical to mock (it makes real
  `gh` subprocess calls), factor the audit-gate decision into a small pure helper
  (`fn is_duplicate_processed(count: i64) -> bool { count >= 1 }`) and unit-test
  that plus the query wiring, keeping network calls out of the test — mirroring how
  the existing handler tests only exercise pure parse/format functions
  (`ci_success_handler.rs:790+`), not the networked entry point.
- **AC5 regression (storm):** a test that calls `try_dedup_check_suite` 8× for one
  `(repo, branch, head_sha)` within the window and asserts exactly one `false`
  (i.e. ≤1 downstream verdict path taken). This is the deterministic, no-network
  proxy for "fire 8 identical events → ≤1 verdict". A true end-to-end integration
  firing 8 messages through mika-qa is out of scope (requires a live agent +
  gateway) and is covered operationally by AC4.

### R5 — Documentation
- Extend the `ci_success_handler.rs` module `## Invariants` header with a short
  note on the dedup layers (in-memory precise gate + audit-durable gate) and the
  `head_sha`-keyed semantics.
- Add a one-line pointer in `crates/mika-agent/CLAUDE.md` (server section) noting
  the new `check_suite_dedup` module and the `ci_success_dedup.skip` /
  `ci_success_handler.dedup_skip` log signals for operator grep (AC4). Run
  `scripts/sync-agent-docs.sh` only if a `docs/`-tracked file changed (this change
  is in a crate-local CLAUDE.md, so likely no sync needed — verify with the
  `docs-sync` CI job expectation).

## Out of scope
- **AC3 in the ticket (mika-qa mailbox backpressure)** — explicitly deferred by
  the ticket ("Defer this to a separate ticket unless AC1+AC2 empirically
  insufficient"). File a follow-up only if AC4 post-deploy data shows the flood
  persists.
- **cpp#98 (compound-bash policy-deny driving op-proxy un-drafts)** — the upstream
  cascade trigger, tracked separately. Both must land to fully retire the op-proxy
  un-draft path, but this ticket's fix stands alone (it dedups the cascade
  regardless of what triggers it).
- Changing `webhook_queue.rs` deferral logic or the gateway's per-workflow event
  emission — the dedup is additive and lives in the agent-side handler.

## Verification contract
- `cargo build -p mika-agent` clean.
- `cargo test -p mika-agent check_suite_dedup` — all new unit tests pass.
- `cargo test -p mika-agent ci_success` — existing handler tests still pass
  (no regression to parse/format/merge-path behavior).
- `cargo clippy -p mika-agent` clean (no new warnings; the DashMap `LazyLock`
  pattern and the eviction sweep must not trip `clippy::type_complexity` etc.).
- `cargo fmt` clean.
- Manual reasoning trace: a burst of 8 identical `(repo, branch, head_sha)` events
  → first passes the in-memory gate, writes the audit marker, runs the full path;
  the other 7 hit either the in-memory gate (`ci_success_dedup.skip`) or, across a
  restart, the audit gate (`ci_success_handler.dedup_skip`) → exactly one merge
  evaluation.
- Two genuinely distinct pushes to the same branch (different head_sha) within 60s
  → both pass (invariant preserved).

## Definition of Done
- New `check_suite_dedup` module with `try_dedup_check_suite`, bounded+TTL DashMap,
  registered in `server/mod.rs`.
- AC1 wired after `find_open_pr` in `try_handle_ci_success`, emitting
  `ci_success_dedup.skip`.
- AC2 audit early-return + `ci_success_handler_processed` marker (head_sha-keyed),
  fail-open on audit-read error.
- Unit tests: first/dup, distinct-head_sha, window-expiry, 5-thread concurrency,
  eviction, 8× storm proxy, audit-gate helper. All green.
- Existing handler tests green; build/clippy/fmt clean.
- Module-header invariant note + CLAUDE.md operator-grep pointer updated.

## Acceptance criteria

1. **AC1 — Webhook dedup module.** New `crates/mika-agent/src/server/check_suite_dedup.rs`
   exposes `try_dedup_check_suite(repo, branch, head_sha, window) -> bool`, backed
   by a thread-safe bounded (~1000-entry) DashMap with 10-min TTL eviction and a
   60s default dedup window. Returns `true` for a repeat `(repo, branch, head_sha)`
   within the window, `false` (and registers the key) on first sighting. Wired into
   `try_handle_ci_success` after `find_open_pr`, emitting an `ci_success_dedup.skip`
   INFO log on skip.
2. **AC2 — Handler audit-events early-return.** `try_handle_ci_success` writes an
   `ci_success_handler_processed` audit row keyed
   `pr:{repo}#{pr_number}@{head_sha}` on first real processing, and before that,
   queries `count_recent_audit_events_for_target("ci_success_handler_processed",
   "pr:{repo}#{pr_number}@{head_sha}", now-60s)`; if `>= 1` it returns
   `Passthrough` with an `ci_success_handler.dedup_skip` log. Audit-read errors
   fail open (proceed, warn).
3. **AC3 — Tests.** Unit: 5 concurrent `try_dedup_check_suite` with the same key →
   exactly one non-duplicate. Unit: handler audit-gate helper returns "skip" when a
   prior marker exists without touching the network. Distinct-head_sha and
   window-expiry cases covered.
4. **AC4 — Post-deploy verify (operational, 24h).** After deploy: mika-qa
   `rate_limit_trip` count drops to `<5/h` (from 41/h baseline); `grep
   ci_success_dedup.skip $MIKA_SPIRIT_LOG_FILE` shows the dedup firing; mika-qa
   verdict audit events resume (`>0` sustained per hour); PRs stop accumulating in
   `REVIEW_REQUIRED` for `>30min` post-CI-green.
5. **AC5 — Regression test for rate-limit storm.** A deterministic test fires 8
   identical `(repo, branch, head_sha)` dedup calls within the window and asserts
   exactly one is treated as non-duplicate (≤1 downstream verdict path), the
   no-network proxy for "8 identical check_suite events → ≤1 verdict".

## Risk / blast radius
**Low.** The dedup is purely additive and subtractive-of-work: a dedup hit only
ever *skips* to `Passthrough` (the no-op path the handler already returns in many
branches), never introduces a new merge path. Worst case of an incorrect
dedup-hit is a delayed verdict, self-healing because any subsequent CI activity on
a live PR re-fires `check_suite.completed` within seconds and the next event
(outside the window, or with the same state) re-evaluates. Zero data loss, no new
merge authority. Fail-open on the audit query ensures a DB hiccup can never block a
legitimate merge. Head_sha-keyed matching guarantees genuine distinct pushes are
never conflated.
