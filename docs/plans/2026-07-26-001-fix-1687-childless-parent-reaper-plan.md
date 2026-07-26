# Plan — fix(task-engine): childless-parent reaper — deterministic reap for silent pilot death (mika#1687)

**Ticket:** mika#1687 (`observation(callback-watchdog): silent pilot death`)
**Type:** fix (loop substrate — tier 2 missing detection coverage; `bug`, `agent-core`, `p2-normal`)
**Branch:** `observation/1687/callback-watchdog-silent-pilot-death-2`
**Target files:** `crates/mika-agent/src/task_engine/engine.rs`, `crates/mika-agent/src/db.rs`,
`crates/mika-common/src/config.rs`

---

## Context

### The investigation (Phase 1 — resolved statically)

mika#1687 filed an `observation`-class datapoint (n=2, 2026-06-30): two mika-dev parent tasks
(`f76f594e`, `0079c47e`) sat `in_progress` for **1h40m / 3h** with **zero callback children** and no
telemetry — well past the agent-loop 5-min deadline, the watchdog's 120s grace, and every reaper grace
window. The ticket asked which of three hypotheses applied and left AC2–AC4 conditional on that answer.

**Those two rows are 26 days stale — a row-level re-investigation is no longer possible** (the live
tasks were long since cleaned/superseded; the server-log window has rotated past them). What *is* still
fully answerable — and is the durable value of the ticket — is a **static coverage analysis of the three
deterministic reaping mechanisms against the observed signature** (parent `in_progress` + **zero callback
children** + aged past all grace windows). That analysis is code-provable today and yields an unambiguous
verdict.

**Verdict: the signature maps to hypothesis 3 (dispatch reached `in_progress` but no callback child /
`process_id` was ever recorded under the parent), and it is a genuine, code-provable coverage gap.** All
three deterministic mechanisms structurally require a callback child to exist:

| Mechanism | Entry query | Requires a callback child? | Blind to zero-child parent? |
|---|---|---|---|
| **Callback watchdog** (#959) — `check_callback_process_liveness` (`engine.rs:482`, wired `:232`) | `get_active_callback_tasks_with_pid` (`db.rs:6402`) — `trigger_type='callback' AND status='in_progress' AND process_id IS NOT NULL` | **Yes** — monitors the *callback child's* PID | **Yes** |
| **Orphan reaper** (#871) — `reap_orphaned_parent_tasks` (`engine.rs:661`, wired `:248`) | `find_orphaned_parent_tasks` (`db.rs:6228`) — `JOIN tasks child ON parent.id = child.parent_task_id … child.status='delivered'` | **Yes** — INNER JOIN + `delivered` child | **Yes** |
| **Parent auto-completer** (#1162) — `complete_parent_tasks_on_callback_success` (`engine.rs:837`, wired `:254`) | `find_completable_parent_tasks_on_pr_url` (`db.rs:6291`) — same INNER JOIN + `delivered` child + `pr_url` | **Yes** — INNER JOIN + `delivered` child | **Yes** |

The `JOIN tasks child ON parent.id = child.parent_task_id` in both reaper queries is an **INNER JOIN**: a
parent with zero children produces zero rows, so neither reaper ever sees it. The watchdog keys off the
callback child's `process_id`, which never exists when no child was spawned. A parent that reaches
`in_progress` without a callback child therefore falls through **all three** deterministic backstops and
sits forever — exactly the "silent" failure Mika Prime named as the scariest concern.

The **only** existing coverage of this state is *soft*: the heartbeat anomaly detectors `stale_pending`
(#583, `db.rs:5836` — `pending` + no callback child) and `dispatch_stale` (#980, `db.rs:5981` —
`in_progress` + no dispatch attempt in >1h) inject a `<task-health>` block into a heartbeat turn. That is
LLM-dependent and best-effort: it surfaces the anomaly to the model as prose but performs **no
deterministic state transition and writes no distinct telemetry**. When the heartbeat turn doesn't act
(the common case for a wedged substrate), the task stays `in_progress` and invisible. There is no
deterministic writer that transitions the parent to `failed` with a discriminable `error_reason` an
operator or monitor can grep.

Hypotheses 1 (genuinely-stuck live subprocess) and 2 (watchdog fired but DB write lost) are **not**
consistent with the evidence: both require a callback child to exist (a live PID for H1; a
watchdog-monitored callback row for H2), and the ticket's own query proved **zero** children. The
zero-child signature is dispositive for hypothesis 3.

### Scope decision — promote observation → substrate fix (deliberate, justified)

The ticket is labelled observation-class ("not yet substrate-fix"), with AC4 anticipating a *follow-up*
substrate ticket if hypothesis 3 held. Given (a) the ticket is 26 days stale, (b) the gap is now
code-provable rather than speculative, and (c) the prime directive ranks *"missing detection coverage —
silent failures the monitor can't see"* as **tier 2 (slows the loop)**, filing yet another observation
ticket to re-derive a conclusion already reached here would be pure churn. **This plan therefore folds
AC4's follow-up into this ticket: it closes the gap with a deterministic childless-parent reaper.** This
promotion is called out explicitly so the architect can accept it or push back — it is a conscious
widening of the ticket's stated scope, grounded in the code analysis above, not a silent one.

The new reaper's job is **fail-with-telemetry**, not re-drive. Re-driving stuck tickets already lives one
layer up: mika#1824's auto-pull stuck-ready reconciler re-kicks tickets that hold `ready` but were never
dispatched. Clean separation: **#1824 rescues at the ticket/label layer; this reaper makes the silent
task-engine death *visible and terminal* at the engine layer**, so the parent stops occupying an
`in_progress` slot and emits a greppable signal.

---

## Requirements

- **R1** — Add a deterministic periodic reaper `reap_childless_stuck_parent_tasks()` to the task engine,
  wired into the same 60-tick DB-scan block as the existing two reapers (`engine.rs:246–254`), invoked
  **after** `complete_parent_tasks_on_callback_success` so success/failure-of-delivered cases are
  resolved first.
- **R2** — It selects parent tasks that are: `status='in_progress'`, `source='self_dev'`,
  `trigger_type='manual'`, `type='issue'`, have **zero** child tasks
  (`NOT EXISTS (SELECT 1 FROM tasks child WHERE child.parent_task_id = parent.id)` — the exact complement
  of the other reapers' INNER JOIN), and whose `updated_at` is older than a configurable grace window.
- **R3** — Grace window is `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` (default **1800**, 30 min). It is
  intentionally far larger than `REAPER_GRACE_SECONDS` (600) because a legitimately-dispatching parent is
  childless only for the sub-second window between the `pending → in_progress` transition and the callback
  child row commit inside `spawn_long_running_exec()`; 30 min removes any plausible in-flight false
  positive (the ticket's real cases were 100–180 min old). Invalid/negative env values fall back to the
  default (WARN-logged), mirroring `stuck_ready_threshold_secs()`.
- **R4** — On match, transition the parent `in_progress → failed` via the terminal-state-guarded
  `update_task_failed` (`db.rs:5355`), with the **distinct** `error_reason`
  **`stuck_in_progress_no_callback_child`** — not the watchdog's `subprocess_exited_without_delivery`,
  not the reaper's `callback_delivered_without_pr_url`. The distinct string is the operator/monitor
  discriminator for this failure class.
- **R5** — Emit an `audit_events` row via `log_audit_event` under a **new sole-writer** `tool_name`
  **`task_engine_childless_reaper`** (before/after = `in_progress`/`failed`, reason =
  `stuck_in_progress_no_callback_child`, with `trace_id`). Using a new tool_name keeps the existing
  `task_engine_reaper` and `task_engine_parent_completer` SOLE-WRITER contracts intact.
- **R6** — Emit a structured INFO `task_engine_childless_reaper.reaped` log (`parent_id`, `agent_id`,
  `created_at`, `age_minutes`, `trace_id`). Reuse `get_reaper_child_snapshot` (`db.rs:6340`) for an
  `evaluated` log that confirms `children_count == 0` at decision time (diagnostic parity with the
  #1126 reaper snapshot; here it documents the *absence* the reaper acted on).
- **R7** — `Ok(false)` from `update_task_failed` (parent already left `in_progress` — operator/agent race)
  → DEBUG skip. `Err` → WARN + an error audit event (`reaper_db_error: …`), mirroring the orphan reaper's
  F5 error-audit behavior (`engine.rs:795–814`). Query error at the top → WARN + return (one bad tick
  never stalls the engine).
- **R8** — Document `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` in `mika/CLAUDE.md`'s env section (under the
  existing "Optional (callback watchdog)" neighbourhood) and add the new reaper + its SOLE-WRITER audit
  event to the § Unified Task Engine reaper-family prose.

---

## Design decisions

### D1 — Mirror the reaper family; invert the child-existence predicate

The new DB query `find_childless_stuck_parent_tasks(agent_id, grace_seconds)` is a near-copy of
`find_orphaned_parent_tasks` (`db.rs:6228`) with two changes: (1) the `JOIN tasks child …` becomes
`NOT EXISTS (SELECT 1 FROM tasks child WHERE child.parent_task_id = parent.id)`; (2) the staleness key is
`parent.updated_at` (there is no delivered-child `updated_at` to key on), compared with
`strftime('%Y-%m-%dT%H:%M:%SZ','now', ?2)`. The `pr_url` predicate, the `dispatch_class` filter, and the
`delivered`-child clauses all disappear (they presuppose a child). Keeps `agent_id`, `status`, `source`,
`trigger_type` filters identical. Add `type = 'issue'` (see D2). Returns a small `ChildlessStuckParent`
struct (`id`, `agent_id`, `created_at`, `updated_at`). The async wrapper on `AsyncDatabase` injects
`self.agent_id()` exactly like `find_orphaned_parent_tasks`'s wrapper.

### D2 — Scope to `type='issue'` in v1; milestone/project deferred

Milestone/project parents (`type IN ('milestone','project')`) legitimately sit `in_progress` between
child dispatches and have their own advancement backstops (`PostCallbackAdvance` #991, the milestone
callback-advance guard #6b, the milestone-context webhook guard #1218). A childless-but-aged milestone is
a *different* stuck-shape with different recovery semantics. Restricting v1 to `type='issue'` matches the
ticket's evidence exactly (both stuck tasks were issue-type dev dispatches) and avoids mis-failing a
milestone mid-orchestration. Milestone/project childless-stuck detection is called out as an explicit
follow-up rather than silently folded in.

### D3 — Fail, don't re-drive (layer separation)

The reaper transitions to `failed` only. It does **not** re-dispatch, re-label, or touch GitHub. Re-drive
is mika#1824's job at the auto-pull/label layer, which re-kicks the *still-open ticket* (the failed task
frees the slot; the ticket keeps its labels and is re-pickable). Coupling fail + re-drive in one place
would duplicate #1824's circuit-breaker/idempotency and risk a fail→redispatch→fail loop. Keeping the
engine reaper single-purpose (make the death visible + terminal) is the orthogonal choice.

### D4 — Distinct `error_reason` and a new SOLE-WRITER audit tool_name

`stuck_in_progress_no_callback_child` is a new, unique `tasks.result` value and
`task_engine_childless_reaper` a new, unique `audit_events.tool_name`. This preserves the documented
SOLE-WRITER contracts of the sibling mechanisms (each reason/tool_name has exactly one writer — see
CLAUDE.md § Unified Task Engine) and gives the monitor a clean, unambiguous signal to count silent-pilot
deaths, distinct from delivered-without-PR orphans and dead-PID subprocess deaths.

### D5 — Grace via env with safe fallback

`childless_parent_reaper_grace_secs()` reads `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS`, parses to `i64`,
falls back to `CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS = 1800` on missing/invalid/≤0 (WARN on
invalid). Same shape as the ticket-adjacent `stuck_ready_threshold_secs()` reader and the watchdog's
`effective_callback_watchdog_grace_period_secs()` (`config.rs:1318`). Kept as a free function next to the
reaper (env read once per tick is negligible at 60s cadence).

### D6 — Placement and cadence

Wire the call at `engine.rs:255` (immediately after `complete_parent_tasks_on_callback_success`, before
`reap_orphaned_team_runs`) so the delivered-child success/failure paths resolve first and only genuinely
childless parents reach this reaper. Same `DB_SCAN_INTERVAL_TICKS` (60) cadence as the whole reaper
family — no new timer.

---

## Implementation steps

1. **Config reader** (`engine.rs`, near `REAPER_GRACE_SECONDS`): add
   `const CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS: i64 = 1800;`,
   `const CHILDLESS_PARENT_REAPER_GRACE_ENV: &str = "MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS";`, and a
   `fn childless_parent_reaper_grace_secs() -> i64` (env parse → default, WARN on invalid/≤0).
2. **DB struct + query** (`db.rs`): add `struct ChildlessStuckParent { id, agent_id, created_at, updated_at }`
   and `fn find_childless_stuck_parent_tasks(&self, agent_id: &str, grace_seconds: i64) -> Result<Vec<ChildlessStuckParent>>`
   (D1 SQL). Add the `AsyncDatabase` async wrapper mirroring `find_orphaned_parent_tasks`'s wrapper
   (injects `self.agent_id()`).
3. **Reaper method** (`engine.rs`): `async fn reap_childless_stuck_parent_tasks(&self)` — query with the
   D5 grace; for each candidate: generate `trace_id`, `system-{agent_id}` session; snapshot children via
   `get_reaper_child_snapshot` and INFO `…​.evaluated` (assert-log `children_count`); `update_task_failed(&id, "stuck_in_progress_no_callback_child")`; on `Ok(true)` write the
   `task_engine_childless_reaper` audit event + INFO `…​.reaped` (with `age_minutes`); on `Ok(false)`
   DEBUG skip; on `Err` WARN + error audit event (R7).
4. **Wire it** at `engine.rs:255` (D6).
5. **Docs** (R8): `mika/CLAUDE.md` env section + § Unified Task Engine reaper-family prose (new reaper,
   new SOLE-WRITER audit `tool_name`, new `error_reason`).
6. **Tests** (see Verification contract).

---

## Verification contract

- `cargo test -p mika-agent task_engine` and `cargo test -p mika-agent childless` — new + existing green.
- `cargo clippy -p mika-agent -- -D warnings`, `cargo fmt --check`.
- **DB-query unit tests** (`db.rs`, mirroring `test_dispatch_stale_*` and the reaper query tests): seed
  an in-memory `Database` and assert `find_childless_stuck_parent_tasks`:
  - **selects** a parent that is `in_progress` / `self_dev` / `manual` / `type='issue'` / zero children /
    `updated_at` older than grace;
  - **excludes** a parent with any child row (INNER-JOIN complement holds);
  - **excludes** a parent younger than grace;
  - **excludes** `type='milestone'` / `type='project'` parents (D2);
  - **excludes** non-`self_dev` / non-`in_progress` / non-`manual` parents.
- **Grace-reader tests**: default on unset, parse on valid, default+WARN on invalid/≤0.
- **Terminal-state race test**: a parent already `failed`/`completed` yields `Ok(false)` from
  `update_task_failed` and is not double-written (reuse the existing `update_task_failed` guard test
  pattern).

### Post-deploy signals (operator)

- `grep task_engine_childless_reaper.reaped $MIKA_SPIRIT_LOG_FILE | jq 'select(.agent_id=="mika-dev")'`
  — each line is one silent-pilot death made visible + terminal. Steady-state expectation: **near-zero**;
  a nonzero-but-bounded count on the first post-deploy day is the backfill of pre-existing wedges.
  **Sustained >5/day** ⇒ the *upstream* dispatch path is producing childless `in_progress` parents at
  rate — escalate to a dispatch-layer root-cause ticket (this reaper is the visibility/terminal backstop,
  not the primary fix for whatever creates the childless parent).
- SQL cross-check: `SELECT * FROM audit_events WHERE tool_name = 'task_engine_childless_reaper';` —
  greppable ledger of the class, distinct from `task_engine_reaper` (delivered-without-PR) and the
  watchdog's `subprocess_exited_without_delivery`.

---

## Definition of Done

- `reap_childless_stuck_parent_tasks` runs every 60-tick DB scan after the two existing reapers (R1, D6).
- Selection matches R2 (zero-child complement of the INNER JOIN) scoped to `type='issue'` (D2), aged past
  `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` (default 1800; R3).
- Match transitions parent → `failed` with distinct `stuck_in_progress_no_callback_child` reason (R4) and
  emits the new SOLE-WRITER `task_engine_childless_reaper` audit event + `.reaped` INFO (R5, R6).
- Race/error paths handled per R7.
- `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` documented in `mika/CLAUDE.md`; reaper family prose updated
  (R8).
- DB-query + grace-reader + terminal-race unit tests pass; clippy/fmt clean; existing reaper tests
  unchanged.

---

## Acceptance criteria

Transcribed verbatim from mika#1687, annotated with resolution (the ticket's ACs are conditional on the
hypothesis; Phase 1 resolves the signature to **hypothesis 3**):

- **AC1** — Investigation documented in PR/comment: which hypothesis (1, 2, or 3) applies to f76f594e +
  0079c47e. May be different for each.
  → **Satisfied.** The § Context investigation resolves the zero-child signature to **hypothesis 3** for
  both (the 26-day-stale rows preclude per-row re-inspection, so the analysis is the code-provable
  coverage-gap verdict; hypotheses 1 and 2 are excluded because both require a callback child the
  evidence proved absent). This plan and the PR body carry the write-up.
- **AC2** — IF hypothesis 1 (genuinely stuck): file a follow-up substrate ticket for the agent-loop-stuck
  class with the trace evidence.
  → **N/A.** Signature is not hypothesis 1 (no live PID / callback child existed).
- **AC3** — IF hypothesis 2 (watchdog fired, DB write lost): file substrate fix for the watchdog write
  path.
  → **N/A.** Signature is not hypothesis 2 (watchdog never engaged — it monitors callback children,
  which never existed).
- **AC4** — IF hypothesis 3 (never spawned): trace the dispatch-readiness rejection chain + document
  where the silent drop happened.
  → **Satisfied and extended.** The silent drop is the deterministic-coverage gap documented in
  § Context (all three backstops require a callback child; the childless parent falls through all of
  them, leaving only LLM-dependent heartbeat soft-detection). Rather than only document, this plan
  **closes** the gap with the deterministic `task_engine_childless_reaper` (the promotion justified in
  § Context → Scope decision).

**Plan-derived acceptance criteria (this fix's testable contract):**

- **AC-a** — `find_childless_stuck_parent_tasks` selects exactly the R2 shape and excludes every negative
  case in the Verification contract (unit-tested, no network/DB mocks needed beyond in-memory `Database`).
- **AC-b** — A qualifying childless `in_progress` `self_dev` issue-parent aged past grace is transitioned
  to `failed` with `error_reason = "stuck_in_progress_no_callback_child"` and one
  `task_engine_childless_reaper` audit row.
- **AC-c** — A parent younger than grace, a parent with any child, and milestone/project parents are
  **not** reaped.
- **AC-d** — The other two reapers and the watchdog are behaviourally unchanged (their tests pass
  untouched; the new reaper's selection set is disjoint from theirs by construction — they require a
  child, it requires none).

---

## Out of scope

- **Milestone/project childless-stuck detection** (D2) — different recovery semantics; separate follow-up.
- **Root-causing *why* a parent reaches `in_progress` without a callback child** (the upstream dispatch
  path). This reaper makes the state visible + terminal; if the post-deploy signal shows a sustained
  rate, that root-cause is a distinct dispatch-layer ticket.
- **Re-driving stuck tickets** — owned by mika#1824's auto-pull stuck-ready reconciler at the label layer
  (D3).
- **Unsticking the original f76f594e / 0079c47e rows** — explicitly out of scope per the ticket (and moot
  26 days on).
- **Changing the heartbeat `stale_pending` / `dispatch_stale` soft-detectors** — they remain the LLM-facing
  early-warning; this reaper is the deterministic backstop beneath them, not a replacement.

---

## Risks

- **R-false-positive-inflight** — reaping a parent that is legitimately mid-dispatch (childless for a
  sub-second window). Mitigated by the 30-min grace (D3/R3), ~1800× the real childless window, plus the
  `update_task_failed` terminal-state guard that no-ops if the parent transitions meanwhile.
- **R-scope-promotion-pushback** — the architect may hold that an observation ticket must not grow a code
  fix without operator sign-off. Mitigated by making the promotion explicit and code-grounded in
  § Context; if held, the fallback is to ship Phase 1 (the coverage-gap write-up) and split the reaper
  into its own substrate ticket — but the 26-day staleness argues against that churn.
- **R-milestone-gap** — v1 leaves milestone/project childless-stuck uncovered. Accepted and documented;
  those parents carry their own advancement backstops (#991, #6b, #1218).
- **R-sole-writer-drift** — a future contributor reusing `task_engine_childless_reaper` or
  `stuck_in_progress_no_callback_child` elsewhere would break the discriminator. Mitigated by the
  SOLE-WRITER note in the reaper doc-comment and CLAUDE.md prose (R8).

## References

- mika#1687 (this ticket) — silent pilot death observation, n=2.
- mika#959 — callback process liveness watchdog (`check_callback_process_liveness`, `engine.rs:482`).
- mika#871 — orphan reaper (`find_orphaned_parent_tasks`, `db.rs:6228`; `reap_orphaned_parent_tasks`,
  `engine.rs:661`).
- mika#1162 — parent auto-completer (`find_completable_parent_tasks_on_pr_url`, `db.rs:6291`).
- mika#583 / mika#980 — heartbeat `stale_pending` / `dispatch_stale` soft-detectors (`db.rs:5836`,
  `db.rs:5981`) — the LLM-facing early-warning this reaper deterministically backstops.
- mika#1824 — auto-pull stuck-ready reconciler (the ticket/label-layer re-drive; complementary layer).
- mika#1126 — reaper child-snapshot diagnostics (`get_reaper_child_snapshot`), reused for the
  `.evaluated` log.
- Mika Prime bearing 2026-06-30 ~16:32Z — named silent pilot death "the fourth concern, the scariest one".
