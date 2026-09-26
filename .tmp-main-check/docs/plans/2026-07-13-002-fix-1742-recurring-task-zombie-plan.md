---
type: fix
issue: 1742
title: Refuse-to-zombie guard on create_recurring_task_if_absent — block silent re-registration after terminal failure
status: draft
---

# Plan — mika#1742 recurring-task zombie (Problem B)

## Ticket

mika#1742 — `create_recurring_task_if_absent` silently re-registers recurring
labels whose most recent instance ended in a terminal-failure state
(`failed`/`cancelled`/`expired`). Root-claude's 2026-07-07 forensic on
`~/.mika/data/mika.db` showed 8 dead `curator_review` rows for `agent_id='mika'`
across 8 days — a fresh UUID created on every mika-spirit restart because the
partial unique index `idx_tasks_unique_recurring` explicitly excludes those
terminal states. Every restart re-triggered the fresh registration; every fresh
registration failed the same way. This ticket lands Problem B (the source-level
guard). Problem A (why Mika's `curator_review` fails specifically) is tracked
separately — root-claude's note attributes it to the PR#1726 RouteFuture/dashmap
wedge, which was already merged before this fix.

## Problem

`create_recurring_task_if_absent` (`crates/mika-agent/src/db.rs`) is called from
`crates/mika-agent/src/server/mod.rs` on every mika-spirit startup, for every
managed agent. Its idempotency is driven by the partial unique index:

```
idx_tasks_unique_recurring
  WHERE trigger_type = 'recurring'
    AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered')
```

An `INSERT OR IGNORE` against that index treats any row in a terminal state as
absent. Result: after the first `failed`/`cancelled`/`expired`, the very next
mika-spirit restart creates a new UUID and dispatches again. If the underlying
cause is not resolved, the new instance also dies. On the eighth restart, eight
zombie rows exist. Mika's `curator_review` was the observed victim; the class
applies to every recurring label.

## Scope

**In scope (this PR):**

1. Add a pre-insert refuse-to-zombie guard on `create_recurring_task_if_absent`.
   When any recurring row for the same `(agent_id, label)` ended in
   `failed`/`cancelled`/`expired` within a `RECURRING_ZOMBIE_GRACE_HOURS`
   window (24h), refuse to re-register, log a `warn!` naming the previous
   task id / status / updated-at, and return `Ok(None)`.
2. Two module-scope consts kept in sync: `RECURRING_ZOMBIE_GRACE_HOURS: u32 = 24`
   and `RECURRING_ZOMBIE_GRACE_SQL: &str = "-24 hours"` (the SQLite `strftime`
   modifier form, kept as a bindable `&str` parameter).
3. Unit tests covering: fresh install registers; active row stays idempotent;
   recent-failed refuses; recent-cancelled refuses; outside-grace-window
   re-registers; guard scoped to label; guard scoped to agent; const-pair
   stays in sync.

**Out of scope (not this PR):**

- Problem A — why Mika's `curator_review` dispatch specifically fails. Root's
  diagnosis attributes it to the PR#1726 wedge (already merged); the next
  natural fire cycle is the regression test.
- Making `RECURRING_ZOMBIE_GRACE_HOURS` runtime-tunable via env var. Kept
  compile-time until real operator experience surfaces a need.
- Auto-cleanup of the eight existing zombie rows. Operator can `mika tasks
  cancel` them manually; the guard prevents new ones accumulating.

## Acceptance criteria

- [ ] **AC1** — `create_recurring_task_if_absent` runs a pre-insert query for
      any recurring row with `(agent_id, label)` matching, status in
      `('failed', 'cancelled', 'expired')`, and `updated_at` inside the
      `RECURRING_ZOMBIE_GRACE_HOURS` window. When one exists, the function
      returns `Ok(None)` and does not insert.
- [ ] **AC2** — Guard hits emit `tracing::warn!` with structured fields
      `agent_id`, `label`, `previous_task_id`, `previous_status`,
      `previous_updated_at`, and `grace_hours`, plus a message telling the
      operator to investigate via `mika tasks get <prev_id>` before the grace
      window elapses.
- [ ] **AC3** — `RECURRING_ZOMBIE_GRACE_HOURS` (`u32`) and
      `RECURRING_ZOMBIE_GRACE_SQL` (`&str`) are declared as `pub const` in
      `db.rs`. A unit test asserts `format!("-{} hours", HOURS) == SQL` so the
      pair cannot silently drift.
- [ ] **AC4** — Fresh install (no prior rows) → registration succeeds
      (`Ok(Some(id))`). Unit test: `zombie_guard_fresh_install_registers`.
- [ ] **AC5** — Existing `recurring_active` row → second call returns
      `Ok(None)`; no duplicate active row inserted. Existing idempotency
      contract unchanged. Unit test:
      `zombie_guard_active_row_still_idempotent`.
- [ ] **AC6** — Recent `failed` row (1h old) blocks re-registration. Unit
      test: `zombie_guard_recent_failed_refuses_registration`.
- [ ] **AC7** — Recent `cancelled` row (2h old) blocks re-registration. Unit
      test: `zombie_guard_recent_cancelled_refuses_registration`.
- [ ] **AC8** — Old dead row (72h old, outside the 24h grace window) does not
      block; re-registration succeeds. Unit test:
      `zombie_guard_expired_grace_allows_registration`.
- [ ] **AC9** — Guard is scoped to `(agent_id, label)`. A recent-failed row
      for a different label on the same agent, or a recent-failed row on a
      different agent with the same label, does not block. Unit tests:
      `zombie_guard_scoped_to_label` and `zombie_guard_scoped_to_agent`.
- [ ] **AC10** — Regression check: `cargo test -p mika-agent` clean.

## Definition of Done

- [ ] All acceptance criteria above met.
- [ ] `cargo build`, `cargo clippy`, `cargo fmt --check`, and
      `cargo test -p mika-agent` pass locally.
- [ ] PR body links to this plan and to mika#1742.
- [ ] No behavior change to the unique-index-driven idempotency for the
      active-row path — the guard is strictly additive on the terminal-state
      seam.

## References

- Ticket: senara-solutions/mika#1742
- Related (Problem A cause): senara-solutions/mika#1726 (RouteFuture/dashmap
  wedge, merged 2026-07-06)
- Callers: `crates/mika-agent/src/server/mod.rs` (`ensure_recurring_task`
  invocation on every startup, per agent)
- Definition: `crates/mika-agent/src/db.rs`
  (`create_recurring_task_if_absent`, unit-test module)
- Root-claude forensic evidence: 8 dead `curator_review` rows for
  `agent_id='mika'` in `~/.mika/data/mika.db` between 2026-06-29 and
  2026-07-06 (see ticket body).
