# Plan — refactor(mika#1259): extract task_state/ module (mika#1448)

## Phase 0 — Pin

**A. Foundation §6 task_state/ definition:**
> `task_state/` — task lifecycle, status transitions.

**B. Sibling-accretion from prior waves:** **TWO methods accreted** from #1445 dashboard_queries/ GROOMED plan classification:
- `list_manual_tasks` (db.rs:5044) — accreted from #1445 §6 classification (line 5044 → "task_state/ per §6: 'task lifecycle'")
- `list_active_tasks` (db.rs:5194) — accreted from #1445 §6 classification (line 5194 → "task_state/ per §6: 'task lifecycle'")

Both confirmed via grep at lines 5044 and 5194. Sibling-accretion mechanism n=2 confirmed (after #1446 memory/ → 5 accreted, #1449 commitments/ → 1 accreted).

**C. Surfaces body-read (db.rs is 17,645 lines):**

### C.1 — Task struct + sibling structs (db.rs:91-358)

| Symbol | Line | Description |
|---|---|---|
| `pub const VALID_TASK_TYPES` | 91 | issue/milestone/project taxonomy |
| `pub struct Task` | 136 | 30+ fields incl. id, agent_id, parent_task_id, depth, label, trigger_type, cron_expr, action_type, status, type, dispatch_class |
| `pub struct BackgroundTaskCounts` | 181 | background-task tally for dashboard |
| `pub struct OrphanedParentTask` | 189 | orphan detection |
| `pub struct CompletableParentTask` | 206 | parent completion detection |
| `pub struct NewTask` | 232 | task constructor input |
| `pub struct TaskHealthAnomaly` | 340 | heartbeat health anomaly |
| `pub struct TaskHealthSummary` | 354 | heartbeat health rollup |

### C.2 — Task methods (db.rs, 50+ methods)

68 `pub fn` methods matching `task` in db.rs. Grouped by lifecycle phase:

**Creation (5 methods)**: create_task (4607), create_recurring_task_if_absent (4662), get_recurring_task_cron (4696), update_recurring_task_cron (4706), cancel_recurring_task_by_label (4723)

**Read / fetch (8 methods)**: get_task (4776), get_manual_task (4788), count_child_tasks (4818), get_schedulable_tasks (4835), get_child_tasks (5988), get_task_descendants (6006), get_tasks_by_status (6264), get_tasks_by_status_and_label (6271), get_tasks_by_trace_ids (8162)

**Status transitions (12 methods)**: update_task_status (4850), update_task_execution_trace_id (4861), claim_and_fire_task (4872), update_task_completed (4883), update_task_failed (4898), update_task_dispatch_class (4915), write_task_dispatch_rejection (4937), promote_task_completed (4952), update_task_next_fire_at (4963), update_task_rescheduled (4973), cancel_task (4981), update_manual_task_status (5006), mark_task_delivered (5669), try_complete_parent_on_sibling_done (5935)

**Listing / aggregation (3 methods, 2 accreted from #1445)**:
- **`list_manual_tasks` (5044)** — accreted from #1445
- **`list_active_tasks` (5194)** — accreted from #1445
- `count_session_tasks` (5090)

**Discovery / lookup (4 methods)**: find_active_task_by_ref_url (5105), find_active_task_by_pr_url (5127), find_active_task_by_branch (5147), find_active_task_by_label (5165), get_task_depth (5182)

**Health / anomaly (1 method)**: get_task_health_summary (5213)

**Lifecycle housekeeping (6 methods)**: mark_tasks_expired (5514), get_expired_child_task_ids (5526), count_pending_tasks (5541), get_user_visible_tasks (5558), get_background_task_counts (5578), get_active_background_task_count (5599)

**Inject-context / callback (3 methods)**: get_inject_context_tasks (5604), get_undelivered_callback_tasks (5621), get_undelivered_callback_tasks_for_session (5642)

**Orphan / completion detection (2 methods)**: find_orphaned_parent_tasks (5701), find_completable_parent_tasks_on_pr_url (5764)

**Process management (4 methods)**: set_task_process_id (5839), get_expired_tasks_with_process_id (5848), clear_task_process_id (5862), get_active_callback_tasks_with_pid (5875)

**Metadata field operations (3 methods)**: set_task_metadata_field (5895), remove_task_metadata_field (5909), get_task_metadata_field (5921)

**Callback / parent relations (4 methods)**: has_active_callback_tasks_excluding (6047), has_non_deferred_active_callback_child (6075), count_pending_callback_tasks_by_team_run (6090), prune_completed_tasks (6254)

**Tests**: 7+ task tests in db.rs at lines 11126-11605 (`test_create_and_get_task`, `test_cancel_task`, `test_count_pending_tasks`, `test_get_child_tasks`, `test_get_task_descendants_*` family, `test_count_pending_callback_tasks_by_team_run`).

**Total in #1448 scope: 50+ methods (covering 68 task-named pub fns; some are not lifecycle/status-transition and may belong elsewhere — see §C.3) + 8 structs + 1 const array + ~10 tests + 1 separate file (`task_metadata.rs`, 173 LoC).**

### C.3 — `task_metadata.rs` (173 LoC, separate file)

`crates/mika-agent/src/task_metadata.rs` — shallow-merge helper `merge_metadata(base, incoming)` for task `metadata` JSON. Used by both `tools/update_task_status.rs` (agent-facing) and `task_engine/dispatcher.rs` (engine-facing). Single 173-LoC file.

**Classification**: task_state/ owns metadata semantics. The merge_metadata helper is task-lifecycle-domain code that happens to live in its own file. Relocate to `task_state/metadata.rs`.

### C.4 — What stays OUT

- **`task_engine/` directory** (cron.rs, dispatcher.rs, engine.rs, queue.rs, process_kill.rs, process_liveness.rs, types.rs, mod.rs) — engine plumbing (dispatch loop, cron firing, process supervision), distinct from §6 task_state/'s "lifecycle, status transitions" domain. Foundation §6 has no `task_engine/` line — likely retained or absorbed at a future Layer-4 refactor.
- **`task_engine/types.rs` task_status / action_type / health_thresholds const modules** — kept inside task_engine/ because they're consumed by engine dispatch logic. task_state/ imports these constants for status-string consistency.
- **Some task-named methods may NOT be lifecycle/status-transition** — e.g., `count_pending_callback_tasks_by_team_run` is a team-run aggregation. Body-read each at extraction time; if any belongs to a sibling §6 module (commitments/, dashboard_queries/), exclude. Conservatively, this plan claims all 50+ task-named methods; future Wave grooming may reclaim some.
- **Tests in `tests/` directory** (integration tests outside crates/mika-agent/src/) — not moved; only db.rs's inline `#[cfg(test)] mod tests` task tests relocate.

### C.5 — Cross-module dependency direction

| Consumer | Imports from task_state/ | Direction |
|---|---|---|
| task_engine/dispatcher.rs | `crate::task_state::*` (NewTask, Task, status const → already in task_engine/types.rs) | task_engine/ → task_state/ ✓ |
| tools/update_task_status.rs | `crate::task_state::merge_metadata` (post-relocation) | tools/ → task_state/ ✓ |
| server/handlers/ (task surfaces) | `crate::task_state::*` | server/ → task_state/ ✓ |
| agent.rs | `crate::task_state::Task` (e.g., for callback task creation) | agent_loop/ → task_state/ ✓ |
| crates/mika-gateway/ | No direct task_state imports (verified via grep — to be confirmed at implementation) | independent |

One-way fan-in. Pure leaf w.r.t. §6.

### C.6 — Test private-access enumeration (per F2 pattern from #1444)

Following #1444's F2 sharpening pattern:

- **Task tests** depend on `db()` constructor (db.rs's `#[cfg(test)] mod tests` helper) — same concern as #1444's audit.rs. Resolved at implementation time (import or local duplicate).
- **No private-state access beyond `db.conn`** — verified `pub(crate) conn: Connection` at db.rs:869 (already crate-public).

## Hypothesis (committed)

**Extraction shape**: split into 3 files inside `crates/mika-agent/src/task_state/`:

- `task_state/mod.rs` — doc-comment per Foundation §6 + re-exports of public surface (Task, NewTask, struct family, merge_metadata)
- `task_state/tasks.rs` — Task struct + sibling structs + VALID_TASK_TYPES const + 50+ `impl Database` methods + ~10 tests (relocated from db.rs)
- `task_state/metadata.rs` — `merge_metadata` helper (relocated from `task_metadata.rs`; `git mv crates/mika-agent/src/task_metadata.rs crates/mika-agent/src/task_state/metadata.rs`)

Rationale: same multi-file shape as #1444 evidence/ — source distributed across two locations (db.rs giant + standalone task_metadata.rs file). Splitting by concern keeps each file under ~1500 LoC.

LARGEST Wave 2 firing by method count (50+ methods vs #1444 evidence/'s 10 methods + 5-7 predicates).

## Approach (committed)

### A. Create module skeleton

```bash
mkdir -p crates/mika-agent/src/task_state
```

Three files:
- `task_state/mod.rs` (doc-comment + re-exports)
- `task_state/tasks.rs` (struct + methods + tests)
- `task_state/metadata.rs` (merge_metadata helper)

### B. Move task_metadata.rs via git mv (preserves history)

```bash
git mv crates/mika-agent/src/task_metadata.rs crates/mika-agent/src/task_state/metadata.rs
```

Update doc-comment to reference task_state/ context.

### C. Cut Task struct + sibling structs from db.rs → task_state/tasks.rs

Sections to cut: db.rs:91-358 (VALID_TASK_TYPES + 8 structs).

### D. Cut 50+ task methods + tests from db.rs → task_state/tasks.rs

Methods stay as `impl Database` blocks (consistent with #1444/#1445/#1446 pattern). Group by lifecycle phase in the new file.

### E. Update lib.rs

```rust
pub mod task_state;
// REMOVE: pub mod task_metadata;
```

### F. Update call sites

Wide-spread call-site updates needed:

- `task_engine/`: imports of `Task`, `NewTask`, status enum constants → `crate::task_state::*`
- `tools/update_task_status.rs`: `crate::task_metadata::merge_metadata` → `crate::task_state::metadata::merge_metadata` (or via mod re-export: `crate::task_state::merge_metadata`)
- `task_engine/dispatcher.rs`: `crate::task_metadata::merge_metadata` → same as above
- `server/handlers/`: dashboard handlers referencing task surfaces — verify imports at implementation time
- `agent.rs`: callback task creation paths — confirm imports updated

### G. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `grep -rn "use crate::db::\(Task\b\|NewTask\b\|BackgroundTaskCounts\b\|task_metadata\)" crates/ tests/` returns ZERO hits
- `grep -rn "use crate::task_metadata" crates/ tests/` returns ZERO hits

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/task_state/mod.rs` created with Foundation §6 doc-comment (parent AC4). Re-exports: `Task`, `NewTask`, `BackgroundTaskCounts`, `OrphanedParentTask`, `CompletableParentTask`, `TaskHealthAnomaly`, `TaskHealthSummary`, `VALID_TASK_TYPES`, `merge_metadata`.

2. **AC2**: `crates/mika-agent/src/task_state/tasks.rs` contains Task struct + 7 sibling structs + VALID_TASK_TYPES const + 50+ `impl Database` methods + ~10 tests, fully relocated from db.rs.

3. **AC3**: `crates/mika-agent/src/task_state/metadata.rs` contains `merge_metadata` helper, relocated from `crates/mika-agent/src/task_metadata.rs` via `git mv` (history preserved). `task_metadata.rs` deleted.

4. **AC4**: db.rs has NO definitions of moved symbols. `grep -rn "^pub struct Task\b\|^pub struct NewTask\|^pub const VALID_TASK_TYPES" crates/mika-agent/src/db.rs` returns ZERO hits.

5. **AC5**: All call sites updated. `grep -rn "use crate::db::Task\b\|use crate::db::NewTask\b\|use crate::task_metadata" crates/ tests/` returns **ZERO hits across all file types** — Rust source, doc-comments, non-Rust. Per #1444's F1 import-sweep discipline.

6. **AC6**: `crates/mika-agent/src/lib.rs` declares `pub mod task_state;` and removes `pub mod task_metadata;` (parent AC4).

7. **AC7**: `cargo test -p mika-agent` passes (parent AC2). Specific checkpoint: moved tests (`test_create_and_get_task`, `test_cancel_task`, `test_count_pending_tasks`, `test_get_child_tasks`, `test_get_task_descendants_*`, `test_count_pending_callback_tasks_by_team_run`, plus merge_metadata tests) pass in their new home.

8. **AC8**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

9. **AC9**: No behavior change (parent AC3) — pure code relocation, same task semantics, same merge_metadata semantics.

10. **AC10**: History-preservation best-effort. `task_metadata.rs` → `task_state/metadata.rs` via `git mv` preserves full history. `task_state/tasks.rs` chunk-cut from db.rs may not have rename-detection; documented as PR-body limitation (same disclosure as #1444).

## Out of scope

- `task_engine/` directory — engine plumbing, not §6 task_state/
- Refactoring task methods (combining queries, deduplicating, etc.) — pure relocation
- `task_engine/types.rs` task_status / action_type const modules — stay in task_engine/

## Risk

**LARGEST Wave 2 firing — 50+ methods + 8 structs + 1 file rename.** Highest call-site churn so far.

- **db.rs chunk-cut from giant file**: 17,645 LoC source; ~2000+ LoC moving in this extraction. Bounded by `cargo build` per sub-step verification.
- **Cross-module impact**: task_engine/ + tools/ + server/handlers/ + agent.rs all consume task_state types. AC5 grep gate catches any missed import.
- **Sibling-accretion correctness**: list_manual_tasks + list_active_tasks confirmed via line-grep at 5044 + 5194 matching #1445's plan classification table. Low risk.

Risk profile: comparable to #1444 evidence/ but with higher method count. Bounded by same AC5 grep discipline.

## Test plan

1. `cargo build -p mika-agent` clean — after each sub-step (B, C, D, F)
2. `cargo test -p mika-agent --lib` passes
3. `cargo build -p mika-gateway` clean (cross-crate sanity)
4. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
5. `grep -rn "use crate::db::Task\b\|use crate::db::NewTask\b\|use crate::task_metadata\|crate::db::list_manual_tasks\|crate::db::list_active_tasks" crates/ tests/` returns **zero hits across all file types** (per AC5)
6. `grep -rn "^pub struct Task\b\|^pub struct NewTask" crates/mika-agent/src/db.rs` returns ZERO hits (per AC4)
7. Specifically run moved tests: `cargo test -p mika-agent --lib task_state::tasks::tests task_state::metadata::tests`

## Implementation order

1. mkdir + 3 file stubs with doc-comments
2. lib.rs: `pub mod task_state;` + remove `pub mod task_metadata;`
3. `git mv crates/mika-agent/src/task_metadata.rs crates/mika-agent/src/task_state/metadata.rs`
4. `cargo build` — fix immediate import paths (`crate::task_metadata` → `crate::task_state::metadata`)
5. Cut 8 structs + VALID_TASK_TYPES from db.rs:91-358 → task_state/tasks.rs
6. `cargo build` — fix all import paths
7. Cut 50+ task methods from db.rs → task_state/tasks.rs (group by lifecycle phase in new file)
8. `cargo build` — fix remaining task_engine/, tools/, server/, agent.rs imports
9. Cut ~10 task tests from db.rs → task_state/tasks.rs `#[cfg(test)] mod tests`
10. `cargo test -p mika-agent --lib task_state` — verify moved tests pass
11. Full `cargo build && cargo test && cargo clippy` from `crates/mika-agent/`
12. AC4 + AC5 grep verifications
