---
type: bug
module: task_engine
tags: [reaper, dispatch_class, race-condition, observability]
issue: 1126
related: [871, 1118, 1001, 996]
---

# Plan: Reaper fires on groom-class child despite v2 fix (mika#1126)

## Problem

The orphaned-parent reaper (#871, v2 fix in #1118/#1120) fired on parent task
`a6ae6043` whose only delivered callback child has `dispatch_class='groom'`.
The v2 SQL filter `COALESCE(child.dispatch_class, 'implement') = 'implement'`
should exclude groom-class children. Manual replay of the query against the
current DB returns 0 rows, confirming the filter is correct for the static case.

## Root cause analysis

The reaper has **zero observability at decision time** — it logs the parent_id
after the kill but never logs what child state it saw when it decided to reap.
Three hypotheses survive:

### H1: Transient second child with NULL dispatch_class (MOST LIKELY)

The parent could have had a second child (e.g., a deferred-dispatch callback
created by `register_deferred_callback()` in executor.rs) that was created,
delivered, and then removed or promoted between the reaper's read and the
operator's manual query. Deferred callbacks in executor.rs:1334 DO set
dispatch_class, but `dispatcher.rs` creates ALL callback tasks with
`dispatch_class: None` (17 occurrences). If any dispatcher.rs code path
created a callback for this parent, it would have `dispatch_class: NULL`,
and `COALESCE(NULL, 'implement') = 'implement'` would match.

### H2: TOCTOU race on dispatch_class write

If any code path creates the task row first (with NULL class) then updates
dispatch_class in a separate transaction, there's a window where the reaper
can read the NULL value. The current `NewTask` struct sets dispatch_class
atomically at INSERT time in executor.rs, so this only applies to alternative
creation paths.

### H3: Multiple children — filter matches one, operator inspected another

The reaper query uses `MIN(child.id) AS callback_task_id` — it returns the
earliest matching child. If the parent had two children (one implement/NULL,
one groom), the reaper would match on the implement/NULL child. The operator
inspected child `ad50980a` (groom-class), which may not be the child the
reaper matched on.

## Fix strategy

The fix has two layers: **observability** (diagnose future occurrences) and
**defense-in-depth** (prevent the reap even if the query has a gap).

## Implementation steps

### Step 1: Add `task_engine_reaper.evaluated` structured log event (AC-1)

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Before the `update_task_failed` call at line 643, query the child task(s) of
the candidate parent and emit a structured INFO log:

```
task_engine_reaper.evaluated:
  parent_id, parent.status, parent.source, parent.dispatch_class,
  callback_task_id (from the query result),
  all_children: Vec<{child_id, dispatch_class, status, trigger_type, action_type, updated_at}>
```

This uses a new `get_reaper_child_snapshot(parent_id)` DB query that returns
ALL children of the parent (not just the one that matched the reaper filter).
The snapshot is taken inside the reap loop, between the `find_orphaned_parent_tasks`
result and the `update_task_failed` call. This gives us a point-in-time view
of what the reaper saw.

**DB method:** `get_reaper_child_snapshot(parent_id: &str) -> Result<Vec<ReaperChildSnapshot>>`

```sql
SELECT id, dispatch_class, status, trigger_type, action_type, updated_at, label
FROM tasks
WHERE parent_task_id = ?1
ORDER BY id
```

**Struct:** `ReaperChildSnapshot` in `db.rs` next to `OrphanedParentTask`.

### Step 2: Defense-in-depth — re-check child dispatch_class before kill (AC-3)

**File:** `crates/mika-agent/src/task_engine/engine.rs`

After the `get_reaper_child_snapshot` call (step 1), check whether ALL delivered
callback children have `dispatch_class = 'groom'` (or any non-implement value).
If so, skip the kill with a WARN log:

```rust
let children = self.db.get_reaper_child_snapshot(&parent.id).await?;

// Defense-in-depth: re-check child classes at kill time.
// The SQL query should have excluded groom-class children, but if the
// class was NULL at query time and populated since, this guard catches
// the race.
let all_children_non_implement = children.iter()
    .filter(|c| c.trigger_type == "callback"
            && c.action_type == "resume_agent"
            && c.status == "delivered")
    .all(|c| c.dispatch_class.as_deref().unwrap_or("implement") != "implement");

if all_children_non_implement {
    warn!(
        parent_id = %parent.id,
        children = ?children,
        "task_engine_reaper: race detected — all delivered callback children \
         are non-implement class at kill time; skipping reap (mika#1126 guard)"
    );
    continue;
}
```

This is a **secondary guard**, not a replacement for the SQL filter. The SQL
filter prevents the query from returning false positives in the common case.
This guard catches the race window where dispatch_class changes between the
query and the kill.

### Step 3: Confirm single writer (AC-2)

**File:** `crates/mika-agent/src/task_engine/engine.rs` (test module)

Add a compile-time documentation test that greps the source for
`callback_delivered_without_pr_url` and asserts exactly one site. This is
already confirmed manually (engine.rs:644 only), but a test codifies it.

Actually — this is better as a code comment on the `update_task_failed` call
rather than a fragile grep-based test. Add a `// SOLE WRITER: callback_delivered_without_pr_url`
comment and note in the reaper doc-comment that any new writers must respect
the groom-class filter.

### Step 4: Regression test (AC-4)

**File:** `crates/mika-agent/src/db.rs` (test module, near existing reaper tests)

Add `test_find_orphaned_parent_tasks_mixed_children_groom_and_null`:

Setup:
- Parent: in_progress, self_dev, manual
- Child A: dispatch_class = NULL, trigger_type = callback, action_type = resume_agent,
  status = delivered, updated_at past grace period, no pr_url
- Child B: dispatch_class = 'groom', same fields

Assert: parent IS reaped (the NULL-class child matches the filter). This
demonstrates H3 — mixed children where only one matches.

Add `test_find_orphaned_parent_tasks_only_groom_children_not_reaped`:

Setup:
- Parent: in_progress, self_dev, manual
- Child A: dispatch_class = 'groom', delivered, past grace
- Child B: dispatch_class = 'groom', delivered, past grace

Assert: parent is NOT reaped (no implement-class children match).

### Step 5: Engine-level integration test for the defense-in-depth guard

**File:** `crates/mika-agent/src/task_engine/engine.rs` (test module)

Test the full `reap_orphaned_parent_tasks` flow where the SQL query returns a
candidate (simulating the race by inserting a parent with a NULL-class child),
but the defense-in-depth re-check catches it (by updating the child to 'groom'
between find and kill, simulating the race window). This validates the Step 2
guard end-to-end.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `ReaperChildSnapshot` struct + `get_reaper_child_snapshot()` method |
| `crates/mika-agent/src/async_db.rs` | Add async wrapper for `get_reaper_child_snapshot()` |
| `crates/mika-agent/src/task_engine/engine.rs` | Add evaluated log + defense-in-depth guard in `reap_orphaned_parent_tasks()` |
| `crates/mika-agent/src/db.rs` (tests) | Add two new reaper test cases for mixed/groom-only children |
| `crates/mika-agent/src/task_engine/engine.rs` (tests) | Add engine-level race-guard integration test |

## Decisions

1. **Observability-first, not fix-first.** We don't have enough data to know
   the exact root cause. The defense-in-depth guard prevents recurrence while
   the structured logging enables definitive diagnosis on next occurrence.

2. **Re-read at kill time, not transactional SQL.** Wrapping the reaper's
   find-and-kill in a single transaction would prevent the race, but SQLite's
   WAL mode means a long-held read transaction pins the WAL snapshot for all
   readers (mika#636 lesson). The re-read is cheaper and sufficient.

3. **No changes to the SQL filter.** The v2 filter is correct for the static
   case. Adding more SQL complexity doesn't help with TOCTOU races.

4. **No `update_task_dispatch_class` production wiring.** The task-reuse
   pattern (#996) has no production callers yet. If/when it does, the race
   window documented here becomes a design constraint for that feature.

## Out of scope

- Groom-class leak detection (the ticket mentions Option B from #1118 —
  detecting when groom dispatches genuinely fail). Separate concern.
- Fixing the dispatcher.rs `dispatch_class: None` paths. Those paths create
  different task types (not long-running callbacks for run_claude_pilot), so
  they're not involved in the reaper's domain. Logging from Step 1 will
  confirm/deny this if the bug recurs.
