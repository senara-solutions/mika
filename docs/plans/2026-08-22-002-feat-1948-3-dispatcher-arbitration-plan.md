---
type: feat
issue: mika#1948
branch: feat/1948/dispatch-lib-manager-3-dispatcher-exec
labels: [enhancement, p1-important, agent-core]
priority: p1-important
touches:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/src/task_engine/engine.rs
  - crates/mika-agent/src/task_engine/dispatcher.rs
  - crates/mika-agent/src/milestone_manager/types.rs
  - crates/mika-agent/src/milestone_manager/cadence.rs
  - crates/mika-agent/src/milestone_manager/reporter.rs
  - crates/mika-agent/src/milestone_manager/no_dispatch_test.rs
  - crates/mika-agent/CLAUDE.md
  - docs/brainstorms (via mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md)
  - tests/eval/dispatcher_contention.rs
---

# Plan — mika#1948 Porte 2 — 3-dispatcher exec-slot arbitration (mika-dev + mika-manager + operator)

## 0. Position within the Porte batch

This is **Porte 2 of 3** gating mika-manager Phase 2 dispatch authority per
`mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md § 3`.

- Porte 1 (mika#1947) — forge-gate loop-résistance (sibling ticket, groomed in parallel).
- **Porte 2 (this ticket) — contention exec (3-dispatcher arbitration).**
- Porte 3 (mika#1949) — INTERNAL_TOKEN alignment cm-spirit (sibling ticket).

**Scope of this PR:** arbitration PRIMITIVES only. No mika-manager Phase 2 wiring; Phase 2 promotion is a separate ticket that unblocks after all 3 portes discharge. The `mika_manager` dispatcher_source value ships in the schema + type surface, but nothing in the tree WRITES that value from a mika-manager code path today. The Phase 1 lecture-seule invariant (`no_dispatch_test.rs`) is EXTENDED here, not relaxed.

## 1. Problem restated

Today the loop is serialized at the dispatch boundary via `has_active_callback_tasks_excluding(task_id, class)` at `crates/mika-agent/src/skills/executor.rs:1122` (mika#583 + mika#1001 split). The guard is agent-scoped (`WHERE agent_id = ?`) and per-class (`implement` / `groom`). It correctly serializes mika-dev's own dispatches, and correctly serializes deferred-wrapper promotions (mika#1011, mika#1172).

The guard is **dispatcher-blind** — it does not know whether the blocking task originated from mika-dev, mika-manager, or an operator. When mika-manager Phase 2 promotes and three dispatchers exist:

1. **mika-dev** — ready-label dispatcher + wip-rescue (mika#1852) + auto-feeder (mika#1863) + auto-pull (mika#1824).
2. **mika-manager** — Phase 2 recommend+execute (gated on this porte).
3. **operator** — interactive `/mika`, `/mika-spawn`, `mika tasks promote-deferred --override` (mika#1453).

...three consequences follow:

- Contention observability degrades — mika-manager cannot tell Vincent *who* is holding the slot when its own dispatch defers.
- Operator dispatches lose priority — a mika-manager deferred wrapper promoted by `promote_pending_deferred_if_idle` can beat an operator-drafted `pending` task to the slot.
- Lineage-cycle detection weakens — `check_lineage_cycle` walks the `parent_task_id` chain but does not consider dispatcher-source, so a `mika_manager → dev-pilot → callback → dev-groom` chain on the same `(repo, issue)` tuple would only trip if the tuple exactly matches, missing the "source escalation" cycle shape.

**Founding evidence (from ticket body):** 2026-07-27→28 11h idle window (mika#1863 auto-feeder founding incident) — pullable-count was 0 while raw ready-count was ≥1. mika-manager firing during that window WOULD have queued behind the auto-feeder with no arbitration model.

## 2. Design — where the arbitration lives

Four surgical extensions, each on the safe side of the existing arbitration. No behavioral change to mika-dev's current dispatch shape (backward compat via COALESCE default).

### 2.a Schema v49 → v50 — `dispatcher_source` column on `tasks`

Nullable `TEXT` column with CHECK constraint pinning to the three known values.

**Rationale for nullable + COALESCE, not NOT NULL with backfill default:** Pre-v50 rows are all mika-dev by definition (autonomous loop is the only current dispatcher). Rather than a hot migration that rewrites every existing row, treat NULL as an unambiguous "pre-v50, therefore mika_dev" sentinel via `COALESCE(dispatcher_source, 'mika_dev')` at every read site. This keeps the migration O(1), matches the pattern from v33→v34 dispatch_class migration (also nullable + CHECK), and preserves the ability to distinguish "pre-v50 row" from "post-v50 row explicitly written by mika-dev" during forensics.

**Migration sketch** (add `migrate_v49_to_v50` next to the existing v34 pattern):

```rust
fn migrate_v49_to_v50(&mut self) -> Result<()> {
    let version = self.schema_version()?;
    if version >= 50 { return Ok(()); }
    // F1 (arch pass1): fail-fast on unexpected baseline — defends against
    // concurrent schema drift under CI. The idempotency guard above returns
    // on >=50; if we reach here we MUST see exactly v49, otherwise our
    // migration assumption is broken.
    if version != 49 {
        anyhow::bail!(
            "migrate_v49_to_v50 called with unexpected baseline version {} (expected 49) — \
             refusing to apply migration; investigate migration order",
            version
        );
    }
    let tx = self.conn.transaction()?;
    let has_col: bool = tx
        .prepare("PRAGMA table_info(tasks)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == "dispatcher_source");
    if !has_col {
        tx.execute_batch(
            "ALTER TABLE tasks ADD COLUMN dispatcher_source TEXT
               CHECK (dispatcher_source IS NULL
                      OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator'));",
        )?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_dispatcher_source
           ON tasks(agent_id, dispatcher_source, status)
           WHERE dispatcher_source IS NOT NULL;",
    )?;
    tx.execute("INSERT INTO schema_version (version) VALUES (50)", [])?;
    tx.commit()?;
    info!("v49→v50: added dispatcher_source column to tasks (#1948)");
    Ok(())
}
```

Update `CURRENT_SCHEMA_VERSION` from 49 → 50 (`crates/mika-agent/src/db.rs:30`) and the schema-init `INSERT INTO schema_version (version) VALUES (49)` → `VALUES (50)` (`crates/mika-agent/src/db.rs:1188`).

Update the migration chain runner (`crates/mika-agent/src/db.rs:911+`) to include `if version < 50 { self.migrate_v49_to_v50()?; }`.

**Wire the CREATE TABLE tasks column list** at `crates/mika-agent/src/db.rs:1245` — add `dispatcher_source TEXT CHECK (...)` so fresh DBs get the column at creation, not just via migration.

### 2.b Slot guard rejection payload — surface `blocking_dispatcher_source`

At `crates/mika-agent/src/skills/executor.rs:1122`, the `has_active_callback_tasks_excluding` return signature currently produces `Option<(String, String, String)>` = `(blocking_parent_id, blocking_callback_id, blocking_label)`. Extend to return `Option<(String, String, String, Option<String>)>` where the fourth field is `blocking_dispatcher_source`.

Concretely, in `crates/mika-agent/src/db.rs` the SQL query underlying `has_active_callback_tasks_excluding` currently returns `(parent_task_id, id, label)` — add `dispatcher_source` to the SELECT and thread it through the tuple. The `AsyncDatabase` wrapper (`crates/mika-agent/src/async_db.rs:1083`) inherits the new tuple shape via type inference.

At the guard callsite (`executor.rs:1122`), extend the rejection JSON:

```rust
let mut rejection = serde_json::json!({
    "error": "global_dispatch_active",
    "task_id": task_id,
    "dispatch_class": class,
    "blocking_task_id": blocking_parent_id,
    "blocking_callback_id": blocking_callback_id,
    "blocking_label": blocking_label,
    "blocker_kind": blocker_kind,
    "blocking_dispatcher_source": blocking_source,  // NEW — Option<String>
    "reason": format!(...),
});
```

**Fail-open on NULL:** `blocking_source` is `Option<String>` and passes through `serde_json` as `null` when the blocking row predates v50. Consumers (mika-manager Reporter, tests) must treat `null` as unknown, NOT as `mika_dev` — the coalesce happens at the read site (`COALESCE(dispatcher_source, 'mika_dev')`), not here, so the payload preserves the on-disk NULL distinction.

**No behavioral change to the guard itself.** Arbitration remains `(agent_id, dispatch_class)`. Adding the source field is observability, not policy.

### 2.c Operator-priority in `promote_pending_deferred_if_idle`

At `crates/mika-agent/src/task_engine/engine.rs:586`, extend the per-class promotion loop to consult a new async DB query `has_pending_operator_task_for_class(agent_id, class)`. When true, skip promotion for that class this tick.

```rust
async fn promote_pending_deferred_if_idle(&self) {
    for class in DISPATCH_CLASSES {
        match self.db.has_any_active_callback_for_class(class).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => { warn!(error = %e, dispatch_class = class, "..."); continue; }
        }
        // NEW: operator-priority guard — an operator-drafted pending task in the
        // same class wins over a deferred-wrapper promotion this tick.
        match self.db.has_pending_operator_task_for_class(class).await {
            Ok(true) => {
                debug!(dispatch_class = class, "deferred promotion skipped — operator pending task holds priority");
                continue;
            }
            Ok(false) => {}
            Err(e) => { warn!(error = %e, "operator-priority check failed"); }  // fail-open
        }
        self.dispatcher.dispatch_next_deferred_callback_for_class(class).await;
    }
}
```

**Fail-open semantics:** if the operator-priority check errors, we fall through to the existing promotion logic — a stray DB error must not silently strand deferred wrappers. If the operator has NO pending tasks in that class (the overwhelming case), the extra query is a single indexed lookup on the new `idx_tasks_dispatcher_source` index — measured cost is one row read per tick per class (≤ 2 reads).

**mika#1453 override preserved:** `force_promote_deferred_for_class` (`crates/mika-agent/src/db.rs:7566`) is unaffected — it's the operator's ESCAPE hatch, not the automatic path, and is explicitly triggered by `mika tasks promote-deferred`.

### 2.d `check_lineage_cycle` — dispatcher-source axis

At `crates/mika-agent/src/skills/executor.rs:1743`, extend the lineage walk to consider dispatcher-source. Today the cycle check compares `(repo, issue, skill)` tuples. Extend `extract_dispatch_tuple` to also emit `dispatcher_source` from the task row, and the cycle-detection condition to trip when `(repo, issue)` matches AND (either `skill` matches OR the proposed dispatch is a mika-manager escalation of a mika-dev tuple already in the lineage).

**Concretely, tighten the check:** if ancestor lineage contains a task with `(repo, issue, dispatcher_source='mika_manager')`, reject a *child* dispatch whose proposed `dispatcher_source` is also mika-manager and whose `(repo, issue)` matches, regardless of skill. Rationale: a mika-manager-triggered dev-pilot that recursively triggers another mika-manager path on the same ticket within its own lineage is the exact deadlock shape Porte 2 must prevent.

**Preserve fail-open:** if `dispatcher_source` is NULL for both proposed and ancestor (pre-v50 rows), the extended check MUST be a no-op — behavior degrades cleanly to the existing tuple-match rule. Tests must cover the pre-v50-row path.

### 2.e mika-manager cadence + Reporter — contention observability (scaffolding-only)

Extend `Assessment` (in `crates/mika-agent/src/milestone_manager/types.rs`) with a new field:

```rust
pub struct Assessment {
    pub severity: Severity,
    pub recommendation: Recommendation,
    pub alerts: Vec<Alert>,
    pub cross_cutting: Vec<String>,
    /// NEW (Porte 2) — dispatch contention events observed during this cycle.
    /// Phase 1: always empty (no manager-driven dispatch yet); Phase 2 will populate.
    #[serde(default)]
    pub contention_events: Vec<ContentionEvent>,
}

pub struct ContentionEvent {
    pub dispatch_class: String,
    pub blocking_task_id: String,
    pub blocking_dispatcher_source: Option<String>,  // NULL = pre-v50 row
    pub reason: String,
}
```

**F3 (arch pass1) — unambiguous wire format:** the `#[serde(default)]` attribute is REQUIRED, not optional. It guarantees:
1. Empty vec serializes as `[]` (not `null`), so Phase 2 consumers reading with typed `Vec<ContentionEvent>` don't need to handle `Option<Vec<_>>`.
2. Pre-Porte-2 JSON blobs (missing the field entirely) deserialize to `Vec::new()` without error — critical for offline sink files written before this ships that must still parse after upgrade.

Test in `types.rs`:
- `test_assessment_serde_default_empty_contention` — assert that `serde_json::from_str::<Assessment>(pre_porte2_json)` succeeds and yields `contention_events == vec![]`.
- `test_assessment_serde_roundtrip_empty_contention` — assert `serde_json::to_string(&Assessment { contention_events: vec![], .. })` produces `"contention_events":[]` NOT `"contention_events":null`.

Extend `Assessor::assess` to accept an optional `Vec<ContentionEvent>` from the caller (cadence) and thread it into `Assessment`. Phase 1 callers pass an empty vec; the field ships primarily to satisfy AC5+AC6+AC9 scaffolding and give Phase 2 a landing spot.

Extend `Reporter::report` (`crates/mika-agent/src/milestone_manager/reporter.rs:46`) with a `### Dispatch contention` section rendered only when `assessment.contention_events.is_empty() == false`. Silent otherwise (fail-open — nothing to report = nothing rendered).

**Cadence wire (`crates/mika-agent/src/milestone_manager/cadence.rs`):** `run_manager_cycle_with` gains an optional injection point for contention events (defaults to empty). No new outbound side effect. LECTURE SEULE invariant preserved.

**FORBIDDEN_TOKENS unchanged for now** — the `contention_events` field is a passive data channel; nothing in `milestone_manager/**` calls `run_claude_pilot` or writes `dispatcher_source = 'mika_manager'`. See § 2.f for the added forbidden token.

### 2.f no_dispatch_test.rs — Phase-2-promotion invariant lock

Extend `FORBIDDEN_TOKENS` in `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` with the literal string that would appear if any file under `milestone_manager/**` tried to write `dispatcher_source = 'mika_manager'` at the SQL layer (the token as it would appear in a SQL string literal or Rust string):

```rust
const FORBIDDEN_TOKENS: &[&str] = &[
    "run_claude_pilot",
    // ... existing entries ...
    // Porte 2 (mika#1948) — no milestone_manager file may write the
    // dispatcher_source='mika_manager' metadata. Phase 2 promotion will
    // require this token to move to a NEW dispatch-wire file OUTSIDE
    // milestone_manager/**, and this test to be updated atomically.
    "dispatcher_source = 'mika_manager'",
    "dispatcher_source=\"mika_manager\"",
];
```

**Why two variants:** SQL literal (single-quoted) and Rust string literal (double-quoted) are the two shapes the token can take at a call site. Match both. The existing EXEMPT_FILES guard (`no_dispatch_test.rs` itself) already handles this file self-referencing the tokens.

### 2.g Integration test — 3-dispatcher race, `#[ignore]` + env-gated

New file `crates/mika-agent/tests/eval/dispatcher_contention.rs` (add if `tests/eval/` doesn't exist yet, otherwise co-locate). Gated behind:

```rust
#[tokio::test]
#[ignore = "requires MIKA_MANAGER_CONTENTION_TEST=1"]
async fn three_dispatcher_race_no_deadlock() {
    if std::env::var("MIKA_MANAGER_CONTENTION_TEST").is_err() { return; }
    // Seed 5 ready tickets. Spawn 3 concurrent dispatchers that each attempt
    // to dispatch all 5. Assert: (a) no double-dispatch (each ticket
    // exactly-once in the callback table), (b) no deadlock (all 5 reach
    // in_progress within N seconds), (c) operator dispatches never lose
    // to a mika_manager deferred wrapper.
}
```

## 3. Acceptance criteria (verbatim from ticket body, mapped to implementation)

Preserved verbatim per `feedback_interactive_mika_plan_needs_ac_section_no_rename.md`. Each AC → the exact commitment in this plan.

- **AC1** — Schema v49 → v50 migration: `tasks.dispatcher_source TEXT` nullable column with CHECK constraint `IN ('mika_dev', 'mika_manager', 'operator')`. Pre-v50 rows stay NULL (treated as `'mika_dev'` via `COALESCE` for backward compat with existing autonomous-loop invocations).
  → § 2.a. Migration `migrate_v49_to_v50`; `CURRENT_SCHEMA_VERSION = 50`; index `idx_tasks_dispatcher_source`; fresh-DB CREATE TABLE also carries the column; COALESCE default `'mika_dev'` applied at read sites.

- **AC2** — `has_active_callback_tasks_excluding` rejection payload gains a `blocking_dispatcher_source: Option<String>` field. Unit test in `executor.rs` verifies: given two tasks in the same class with sources `mika_dev` and `mika_manager`, the second-attempted dispatch is rejected with `blocking_dispatcher_source = "mika_dev"`.
  → § 2.b. DB query extended; tuple widened; rejection JSON extended; unit test added in `executor.rs` test module alongside existing `has_active_callback_tasks_excluding` tests (lines 4486+).

- **AC3** — `promote_pending_deferred_if_idle()` skips promotion when a pending task with `dispatcher_source = 'operator'` exists in the same class. Unit test in `engine.rs`: seed one deferred wrapper (`mika_manager`) + one pending operator task; assert the wrapper is NOT promoted until the operator task completes.
  → § 2.c. New async DB helper `has_pending_operator_task_for_class`; guard call in the promotion loop; test added alongside existing `test_promote_pending_deferred_if_idle_*` suite (`engine.rs:2691+`).

- **AC4** — `check_lineage_cycle()` extension: given a lineage of `mika_manager → dev-pilot → PR-callback → dev-groom` targeting the same `(repo, issue)`, the second dispatch (`dev-groom`) is rejected with `cycle_detected` referencing the lineage source. Unit test in `executor.rs`.
  → § 2.d. `extract_dispatch_tuple` extended with `dispatcher_source`; walk condition extended to trip on source escalation; test added alongside existing lineage tests (`executor.rs:5595+`).
  **F2 (arch pass1) — explicit fail-open NULL coverage in AC4 tests:** the test suite MUST include a case where BOTH proposed AND ancestor `dispatcher_source` are NULL (pre-v50 rows) and the cycle check proceeds without false trip. Named test: `test_check_lineage_cycle_null_dispatcher_source_fail_open`.

- **AC5** — `mika-manager` cadence: when `run_manager_cycle` detects a dispatch rejection (via a new hypothetical Phase 2 code path — GATED behind Phase 2 promotion, so this AC is scaffolding-only in this PR), the rejection reason including `blocking_dispatcher_source` is threaded into `Assessment.recent_activity`. Unit test in `cadence.rs` with mock rejection.
  → § 2.e. `Assessment.contention_events` field added; `run_manager_cycle_with` accepts optional contention-event injection; test uses direct field injection (no live dispatch attempt — Phase 1 lecture-seule preserved). NB: plan places contention events on the new `contention_events` field per the design's typed-list shape, not on `recent_activity` (which is `MilestoneState.recent_activity: Vec<RecentActivity>` and is GitHub-derived). Discrepancy from AC text noted here — implementer verifies with architect during first-pass; if architect prefers `recent_activity`, we widen `RecentActivity` instead.

- **AC6** — `Reporter` (in `milestone_manager/reporter.rs`) renders a "Dispatch contention" section when `assessment.contention_events.len() > 0`, silent otherwise. Unit test in `reporter.rs`.
  → § 2.e. New `### Dispatch contention` section after "Cross-cutting concerns" in Reporter output; test asserts empty vec → section absent, populated vec → section present with entries.

- **AC7** — `no_dispatch_test.rs` FORBIDDEN_TOKENS updated to explicitly forbid `dispatcher_source = 'mika_manager'` write from any file OTHER than `milestone_manager/**` — Phase-2-promotion invariant lock.
  → § 2.f. Two token variants added (SQL literal + Rust string literal). NB: the ticket text says "OTHER than milestone_manager/**" but the existing test's SCOPE is the milestone_manager subtree itself (it greps files UNDER milestone_manager/). We honor the ticket's INTENT — no milestone_manager file writes this token — which is what the existing subtree-scoped grep already enforces. If a Phase 2 dispatch-wire file lands OUTSIDE `milestone_manager/**`, a separate test at that boundary will be needed then. Called out for architect adjudication.

- **AC8** — Integration test (`tests/eval/dispatcher_contention.rs`, gated behind `#[ignore]` + `MIKA_MANAGER_CONTENTION_TEST=1`): simulates 3-dispatcher race with 5 seeded ready tickets; asserts no double-dispatch, no deadlock, all 5 tickets eventually dispatched exactly once.
  → § 2.g. Test file added with the ignore+env gate; race modeled via tokio spawn of 3 dispatcher-simulator tasks against a shared in-memory DB.

- **AC9** — `crates/mika-agent/CLAUDE.md § Unified Task Engine` updated with a "Dispatcher-source arbitration" subsection documenting the three sources + operator-priority rule.
  → § 2.a-d ownership doc. Subsection added after existing Unified Task Engine content covering: (i) the three sources + COALESCE default, (ii) operator-priority in `promote_pending_deferred_if_idle`, (iii) cycle-check dispatcher-source extension, (iv) rejection-payload `blocking_dispatcher_source` field.

- **AC10** — `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md § 3 Porte 2` updated with a "**Statut : DISCHARGED**" line naming this ticket + PR + AC-satisfaction summary.
  → Post-merge doc update. Because the brainstorm file lives in `mika-platform/` (NOT this mika repo), the update MUST be a separate cross-repo commit made after this PR merges (or as a companion PR to `mika-platform`). Plan notes this explicitly so the implementer does not attempt to edit a file outside the worktree. Suggested wording: `**Statut : DISCHARGED** — via mika#1948 (PR #<N>) — AC1-AC9 landed. Phase 2 dispatch authority may proceed once Porte 1 (mika#1947) + Porte 3 (mika#1949) also discharge.`
  **F4 (arch pass1) — reminder mechanism.** Squash-merge risk on PR-body checklist. Twofold guard:
  1. Enforce the AC10 reminder in the PR body TITLE line: `feat(dispatch-lib,manager): 3-dispatcher exec-slot arbitration (Porte 2) — AC10 post-merge: update mika-platform brief`. The title survives squash and is what the reviewer sees on the merge page.
  2. Add a `// FIXME(mika#1948-AC10)` inline comment in the primary migration file (`crates/mika-agent/src/db.rs` next to `migrate_v49_to_v50`) — a searchable long-lived marker that survives the PR. Implementer removes it in a follow-up PR after landing the mika-platform doc update.

## 4. Out of scope

Verbatim from ticket body:

- Wiring mika-manager Phase 2 dispatch surface itself — this ticket adds the arbitration primitives; Phase 2 flow is a separate ticket that unblocks after all 3 portes.
- Cross-agent contention (e.g., mika-manager for one agent while operator dispatches to another agent) — the per-agent slot scoping already covers this; no cross-agent lock needed.
- Priority classes beyond `operator > mika_manager > mika_dev` (e.g., emergency-preempt) — deferred until real operational need surfaces.
- Fairness across multiple mika-manager cadence cycles for different milestones — Phase 1.5 constraint says ONE milestone at a time; scaling deferred.

## 5. Blocks

Verbatim from ticket body: **mika-manager Phase 2 dispatch authority.** Cannot promote Phase 2 without this + Porte 1 (mika#1947) + Porte 3 (mika#1949) all discharged.

## 6. Sequencing / phases

1. **Phase A — Schema (isolated)**: v50 migration + CURRENT_SCHEMA_VERSION bump + fresh-DB CREATE TABLE + index. Run `cargo test -p mika-agent db::` to confirm migration idempotency + fresh-DB creation. Commit boundary.
2. **Phase B — Read-site COALESCE**: audit all read sites of the `tasks` table; wrap in `COALESCE(dispatcher_source, 'mika_dev')` where the source is read (there are ≤ 5 such sites — the guard, cycle-check, promoter, contention reporter, and the integration test). Commit boundary.
3. **Phase C — Guard payload widening**: extend `has_active_callback_tasks_excluding` tuple + rejection JSON. Unit tests. Commit boundary.
4. **Phase D — Operator-priority**: `has_pending_operator_task_for_class` DB helper + promoter guard + unit test. Commit boundary.
5. **Phase E — Cycle-check extension**: `extract_dispatch_tuple` + cycle condition + unit tests. Commit boundary.
6. **Phase F — Reporter + Assessment surface**: `Assessment.contention_events` + Reporter section + no_dispatch_test.rs FORBIDDEN_TOKENS. Unit tests. Commit boundary.
7. **Phase G — Integration test + docs**: `dispatcher_contention.rs` integration test + `CLAUDE.md` subsection. Final commit.
8. **Phase H — Cross-repo doc (post-merge)**: separate mika-platform commit updating the design brief's Porte 2 to `Statut : DISCHARGED`. Called out here so /ce:work does not attempt in this worktree.

Each phase self-tests via `cargo test -p mika-agent <module>::` before moving on. Full `cargo build && cargo test -p mika-agent` on completion of Phase G.

## 7. Risk register

- **R1 — Migration in production DBs.** Existing v49 DBs must migrate cleanly on next `mika` startup. Mitigation: idempotent guard (`if version >= 50 { return Ok(()) }`) + explicit baseline assert (`version != 49 → bail!` per F1 pass1) + PRAGMA-based column detection. Verified by `test_migration_v49_to_v50_is_idempotent` and `test_migration_v49_to_v50_bails_on_unexpected_baseline` (add).
- **R2 — Existing has_active_callback_tasks_excluding callers.** The tuple widens from 3 to 4 fields. All callers must be updated. Grep confirms only one production caller (`executor.rs:1122`) plus test-file callers. Mitigation: compiler-enforced (breaking type change surfaces every caller).
- **R3 — Deferred wrapper stranded by operator-priority.** If an operator ever leaves a `pending` task drafted-but-never-run, mika-manager deferred wrappers would stall in that class forever. Mitigation: operator-drafted `pending` tasks already have their own liveness contract (they're operator-owned and expected to be started promptly). The auto-feeder + wip-rescue already handle stuck-pending tasks; this arbitration inherits those recovery paths without change.
- **R4 — Cycle-check false-positive.** Extending cycle detection can over-reject. Mitigation: § 2.d's fail-open on NULL dispatcher_source (both proposed and ancestor); the new condition is ADD, not REPLACE — the existing exact-tuple match still fires first, so existing behavior is preserved for the mika-dev-only case.
- **R5 — Reporter section renders when empty vec is `Some(vec![])` vs `None`.** Serde behavior on `Vec<T>` defaults to empty vec = present-but-empty. Mitigation: render condition is `!contention_events.is_empty()`, not `contention_events.is_some()` — no ambiguity.
- **R6 — Cross-repo AC10 forgotten.** Post-merge action lives outside this worktree; implementer might complete /mika and forget. Mitigation (per F4 pass1): (i) PR title carries `AC10 post-merge: update mika-platform brief` — survives squash and reviewer sees it on merge page; (ii) `// FIXME(mika#1948-AC10)` marker inline next to `migrate_v49_to_v50` — searchable long-lived reminder, removed by a follow-up PR after the doc lands; (iii) PR body checklist entry for redundancy.

## 8. Test surface

- `cargo test -p mika-agent db::tests::migrate_v49_to_v50` — migration + idempotency + baseline-bail (F1 pass1).
- `cargo test -p mika-agent skills::executor::tests` — guard payload + cycle check (including `test_check_lineage_cycle_null_dispatcher_source_fail_open` per F2 pass1).
- `cargo test -p mika-agent task_engine::engine::tests` — operator-priority in promoter (both "operator pending → defer" and "no operator → promote normally" cases).
- `cargo test -p mika-agent milestone_manager::types::tests` — serde default + roundtrip on `contention_events` (F3 pass1).
- `cargo test -p mika-agent milestone_manager::reporter::tests` — Dispatch contention section render.
- `cargo test -p mika-agent milestone_manager::cadence::tests` — contention events threading (mock injection).
- `cargo test -p mika-agent milestone_manager::no_dispatch_test` — FORBIDDEN_TOKENS gate.
- `MIKA_MANAGER_CONTENTION_TEST=1 cargo test -p mika-agent --test dispatcher_contention -- --ignored` — integration test.
- `cargo clippy -p mika-agent -- -D warnings` — no new warnings.

## 9. Non-goals (guardrail against scope creep)

- No mika-manager Phase 2 dispatch code. This PR ships the arbitration primitives ONLY; no code path writes `dispatcher_source = 'mika_manager'` today.
- No changes to mika-dev's dispatch behavior. The COALESCE default preserves exact today-behavior for the autonomous loop.
- No changes to operator dispatch behavior. Operator paths (`/mika`, `/mika-spawn`, `mika tasks promote-deferred`) continue to use their existing mechanics; the plan requires implementers to set `dispatcher_source = 'operator'` when tasks are INSERTED from operator surfaces, but that's a metadata-write, not a policy change.
- No changes to Porte 1 (forge-gate) or Porte 3 (INTERNAL_TOKEN) surfaces. Portes are intentionally orthogonal per the design brief.

## 10. References

- Design brief: `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md § 3 Porte 2`.
- Guard site: `crates/mika-agent/src/skills/executor.rs:1122` `has_active_callback_tasks_excluding`.
- Promoter site: `crates/mika-agent/src/task_engine/engine.rs:586` `promote_pending_deferred_if_idle`.
- Cycle-check site: `crates/mika-agent/src/skills/executor.rs:1743` `check_lineage_cycle`.
- Migration precedent: `crates/mika-agent/src/db.rs:4030+` v33→v34 `dispatch_class` (same nullable + CHECK pattern).
- Manager types: `crates/mika-agent/src/milestone_manager/types.rs:198` `Assessment`.
- Reporter: `crates/mika-agent/src/milestone_manager/reporter.rs:46` `Reporter::report`.
- Cadence: `crates/mika-agent/src/milestone_manager/cadence.rs:404` `run_manager_cycle`.
- Invariant test: `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`.
- Prior art tickets: mika#583, mika#1001, mika#1011, mika#1163, mika#1172, mika#1453, mika#1852, mika#1863, mika#1824.
- Founding incident: 2026-07-27→28 11h idle (auto-feeder motivator, mika#1863).
- Sibling ticket grooming: Porte 1 mika#1947 (in flight), Porte 3 mika#1949.
