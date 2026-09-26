---
title: "Tasks.type Column: Orthogonal Work-Item Role for Milestone/Project Dispatch"
date: 2026-04-16
category: architecture-patterns
module: crates/mika-agent/src/db.rs
component: database
problem_type: best_practice
severity: medium
tags:
  - schema-migration
  - work-items
  - orthogonal-design
  - sqlite
  - check-constraint
  - raw-identifier
  - dedup-semantics
related_components:
  - crates/mika-agent/src/tools/create_task.rs
  - crates/mika-agent/src/tools/list_tasks.rs
  - crates/mika-agent/src/tools/check_task.rs
  - crates/mika-agent/src/server/dashboard.rs
applies_when:
  - Adding a new categorical column to a widely-consumed SQLite table
  - Extending a work-item primitive to support new orchestration without coupling core to that orchestration
  - Introducing a tool parameter whose wire name collides with a Rust keyword
---

# Tasks.type Column: Orthogonal Work-Item Role for Milestone/Project Dispatch

## Context

mika-dev needed a way to distinguish work-item *containers* (milestones, projects) from work-item *leaves* (issues), so self-dev could expand a milestone into child issues and dispatch each child via `run_claude_pilot`. Without a first-class type field, the only way to recognize a parent was "it has children" — fragile (a parent with no expanded children yet looks identical to a leaf) and undiscoverable (audit tools had to label-match rather than query).

The constraint was that **mika core must stay a dumb work-item store.** All orchestration logic (milestone expansion, project sprints, auto-close of parents when all children complete) lives in mika-skills/self-dev. mika core only provides the primitive: a column that distinguishes the three roles.

See the [origin brainstorm](../../../../mika-platform/docs/brainstorms/2026-04-15-milestones-and-projects-as-sprints-brainstorm.md) and issue [#595](https://github.com/senara-solutions/mika/issues/595).

## Guidance

When introducing a new categorical column to the `tasks` table — or any widely-consumed SQLite table — follow this pattern:

### 1. Orthogonal column, not parallel table

A `type` column on the existing `tasks` row is cheaper than a sibling `milestones` table:

- Children reuse the existing `parent_task_id` FK — no new edges to maintain.
- Every existing query (`list_tasks`, `check_task`, dedup helpers, dashboard DTOs) continues to work — additive change.
- The agent loop's `validate_dispatch_readiness` stays type-agnostic. Dispatch readiness remains a function of `status` alone.
- Non-manual tasks (callback, recurring, a2a) receive `type='issue'` automatically via the SQL `DEFAULT`, and it stays inert for them — zero cost.

### 2. Single-statement migration with inline CHECK

SQLite 3.37+ (shipped by rusqlite's bundled feature) supports adding CHECK constraints via `ALTER TABLE ADD COLUMN`. No table rebuild needed:

```rust
fn migrate_v22_to_v23(tx: &rusqlite::Transaction) -> Result<()> {
    if !column_exists(tx, "tasks", "type")? {
        tx.execute(
            "ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'issue' \
             CHECK (type IN ('issue', 'milestone', 'project'))",
            [],
        )?;
    }
    tx.execute(
        "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
        params![23, crate::timestamp::now()],
    )?;
    Ok(())
}
```

The `NOT NULL DEFAULT 'issue'` pair backfills every existing row atomically — no separate backfill step, no intermediate state where some rows are NULL. The `column_exists` guard makes the migration idempotent.

### 3. Rust raw identifier (`r#type`) keeps the external surface clean

The SQL column, JSON serialization name, tool parameter name, and response label are all `type`. Rust requires the raw identifier:

```rust
pub struct Task {
    pub id: String,
    // ... other fields
    pub r#type: String,
}
```

With `r#type`, serde auto-derives `"type"` as the JSON field name — no `#[serde(rename)]` needed. The small syntactic cost at construction sites (`NewTask { r#type: "milestone".into(), .. }`) is paid once per site; the external surface stays idiomatic.

### 4. Two-layer validation: tool boundary + DB CHECK

Validate at the tool boundary for an actionable error message to the LLM. Rely on the DB CHECK as defense-in-depth:

```rust
const VALID_TYPES: &[&str] = &["issue", "milestone", "project"];

let task_type = input["type"].as_str().unwrap_or("issue").trim();
let task_type = if task_type.is_empty() { "issue" } else { task_type };
if !VALID_TYPES.contains(&task_type) {
    return Ok(ToolOutput::error(format!(
        "Invalid type '{}'. Must be one of: {}",
        task_type,
        VALID_TYPES.join(", ")
    )));
}
```

Empty-string input is treated as absent (defaults to `'issue'`), avoiding gratuitous errors on whitespace-only payloads. The DB CHECK never fires in practice — it exists to catch direct SQL writes from future code paths that bypass the tool layer.

### 5. Dedup keys intentionally do NOT include type

The existing dedup indexes `(agent_id, reference_url)` and `(agent_id, label COLLATE NOCASE)` identify the *underlying object*, not its role. A milestone and an issue pointing at the same `reference_url` would dedup — and that's correct: they describe the same GitHub issue, just viewed through different orchestration lenses. Including `type` in the dedup key would let an agent accidentally create two rows for the same GitHub issue under different type labels.

In practice, milestone parents and their child issues have distinct `reference_url` values (e.g. `mika#6` for the milestone umbrella and `mika#581` for a child), so collision is not a real-world concern.

### 6. Additive DTO fields are safe by construction

Adding `pub r#type: String` to `TaskResponse` and `TaskDetailResponse` is a non-breaking API change. Serde-based Rust clients and TypeScript consumers that don't reference the field ignore it. The dashboard adopts the field on its own schedule without re-shipping the agent.

### 7. Every hand-written SELECT site is a migration risk

The most fragile part of adding a column to a large table is finding every place that builds a `Task` struct *outside* `row_to_task`. In mika-agent these were:

- `find_active_work_item_by_ref_url` (db.rs:3123) — inline SELECT + manual struct literal
- `find_active_work_item_by_label` (db.rs:3230) — same pattern
- `find_active_work_item_by_pr_url` / `_by_branch` — same pattern

Rust struct literals require all fields, so the **compiler catches these automatically** once you add `r#type` to `Task` — the build fails loudly at every forgotten site. This is structurally bounded risk, which is the point: lean on the type system, don't rely on grep.

Also update:
- `TASK_COLUMNS` const (ordered column list)
- `TASK_COLUMN_COUNT` (29 → 30)
- `create_task` and `create_recurring_task_if_absent` INSERTs
- `row_to_task` ordinal reads

## Why This Matters

- **Future orchestration stays decoupled.** Milestone expansion, project sprints, and parent auto-close can evolve in mika-skills without touching agent core. The schema primitive is stable; policy lives where policy belongs.
- **Zero-touch rollout.** The `NOT NULL DEFAULT 'issue'` migration runs in milliseconds on production DBs with no operator intervention. Existing rows are immediately valid under the new CHECK.
- **Agent prompts don't need retraining for the default case.** An LLM that has never heard of `type='milestone'` keeps working — the parameter is optional and defaults to the old behavior.
- **The "dumb store" discipline pays off long-term.** Every time mika core stays agnostic of a particular orchestration, it remains reusable for the *next* orchestration. The alternative (baking milestone semantics into core) would have forced a coupled migration the next time self-dev invents a new work-item role.

## When to Apply

This pattern applies whenever you need to:

1. Add a categorical distinction to an existing primitive without committing core code to the downstream policy that consumes the distinction.
2. Extend a widely-consumed SQLite table with a constrained-enum column that must preserve backward compatibility for every existing row and every existing query.
3. Introduce a tool parameter whose natural wire name (`type`, `match`, `move`, `mod`) collides with a Rust keyword — the raw-identifier trick cleanly resolves it.

Do **not** apply when:

- The new dimension is relational (would produce many-to-one joins to a lookup table rather than a finite enum).
- The downstream policy is already owned by the same crate (in that case the column can carry policy-specific semantics without violating decoupling).
- Existing rows would need custom backfill logic that can't be expressed as a single SQL `DEFAULT`.

## Examples

### Schema migration checklist (what to update in lockstep)

```
1. Bump CURRENT_SCHEMA_VERSION (v22 → v23)
2. Add migrate_vN_to_vN+1 method with column_exists guard
3. Wire into the migration ladder in run_migrations
4. Update clean-slate migrate_v1 baseline (CREATE TABLE tasks (...))
5. Add the field to Task and NewTask structs (use r#type for Rust keywords)
6. Update TASK_COLUMNS const
7. Update TASK_COLUMN_COUNT
8. Update row_to_task ordinal reads
9. Update create_task INSERT
10. Update create_recurring_task_if_absent INSERT
11. Update every hand-written SELECT + struct literal (compiler catches these)
12. Update DTOs: TaskResponse, TaskDetailResponse (+ their From<Task> impls)
13. Update MEMORY.md Schema Evolution log
14. Update crates/mika-agent/CLAUDE.md schema version
15. Update docs/runtime-structure.md migration table
16. Run scripts/sync-agent-docs.sh (docs-sync CI job enforces this)
```

### Tool response formatting: hide the default, show non-defaults

```rust
// list_tasks: append type only when it differs from the default
let type_segment = if task.r#type != "issue" {
    format!(" type:{}", task.r#type)
} else {
    String::new()
};

// check_task: always show type (single-item inspection benefits from explicitness)
writeln!(output, "Type: {}", task.r#type)?;
```

This keeps output compact for the dominant `type='issue'` case while making milestone/project rows visually distinct in long listings.

## Related Documentation

- [`work-item-tracking-manual-task-reuse.md`](work-item-tracking-manual-task-reuse.md) — Established the `trigger_type='manual'` + `action_type='none'` shape that this column extends orthogonally.
- [`create-work-item-duplicate-on-retry.md`](../logic-errors/create-work-item-duplicate-on-retry.md) — The dedup semantics rationale that informed why `type` is deliberately not a dedup key.
- [`iso8601-timestamp-migration.md`](../database-issues/iso8601-timestamp-migration.md) — Contrasting pattern: a column *type change* across 17 tables required full-rebuild migrations. Column *addition* with `ALTER TABLE ADD COLUMN` is dramatically simpler and this doc explains when to reach for which.
- `docs/plans/2026-04-16-003-feat-tasks-type-column-and-create-work-item-support-plan.md` — Implementation plan with full requirements trace and unit breakdown.
- `docs/runtime-structure.md` — Schema v23 reference.
- Companion ticket: [mika-skills#149](https://github.com/senara-solutions/mika-skills/issues/149) — self-dev milestone/project workflow that consumes this primitive.
