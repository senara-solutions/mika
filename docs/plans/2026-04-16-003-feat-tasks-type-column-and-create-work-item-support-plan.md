---
title: "feat(agent): tasks.type column + create_work_item support"
type: feat
status: active
date: 2026-04-16
origin: ../../mika-platform/docs/brainstorms/2026-04-15-milestones-and-projects-as-sprints-brainstorm.md
issue: senara-solutions/mika#595
---

# feat(agent): tasks.type column + create_work_item support

## Overview

Add a `type` column to the `tasks` table with three valid values (`issue`, `milestone`, `project`) and accept an optional `type` parameter in the `create_work_item` tool. Default to `'issue'` so existing callers and rows are unaffected. This is the foundational schema + tool piece that unblocks mika-dev's milestone/project orchestration (mika-skills#149); mika core stays a dumb work-item store.

## Problem Frame

mika-dev needs to express "this work item is a milestone or project (a parent), not an individual issue" so that:
- Self-dev can fetch the milestone/project, expand it into child `type=issue` work items linked via `parent_task_id`, and dispatch each child via `run_claude_pilot`.
- Audits and TUI can distinguish parent containers ("N of M children completed") from leaf work.
- mika-platform's renamed `mika-milestone-audit` command can navigate the tree by `parent_task_id` + `type=milestone` instead of label matching.

Today there is no way to mark a work item as a parent container. The only differentiator between a milestone parent and a regular issue would be the presence of children — fragile and undiscoverable. See origin brainstorm for full architecture context.

## Requirements Trace

- **R1.** `tasks.type` column added with CHECK constraint `type IN ('issue', 'milestone', 'project')` and DEFAULT `'issue'`. (Issue: Schema)
- **R2.** Existing rows backfill to `'issue'` via the column DEFAULT. (Issue: Schema acceptance)
- **R3.** `Task` and `NewTask` structs in `crates/mika-agent/src/db.rs` carry `type` through to row reads and writes. (Issue: Schema)
- **R4.** `create_work_item` tool accepts an optional `type` parameter (string), defaults to `'issue'`, validates against the three allowed values, and returns a structured error for invalid input. (Issue: Tool)
- **R5.** `list_work_items` and `check_work_item` surface `type` in their output. (Issue: Queries / audits)
- **R6.** Unit tests cover the four `create_work_item` cases (no type, three valid types, invalid type) plus `list_work_items` / `check_work_item` rendering. (Issue: Acceptance)
- **R7.** No new orchestration logic, dispatch guards, or auto-close behavior in mika core. (Issue: Non-goals)

## Scope Boundaries

- No changes to `validate_dispatch_readiness` — it stays type-agnostic. Dispatch readiness still works on status alone.
- No auto-close logic for milestone/project parents. Closing happens explicitly from self-dev.
- No new database query endpoints. Existing `parent_task_id` traversal already supports walking the tree.
- No CLI flag for `mika tasks list` to filter by type in this PR (defer until self-dev actually consumes it).
- No A2A protocol surface change — A2A tasks remain `type='issue'` by default; A2A clients have no concept of milestone/project.

### Deferred to Separate Tasks

- Self-dev milestone/project workflow branches: **mika-skills#149**.
- Retire `/mika-sprint`, rename audit command to `mika-milestone-audit`: **mika-platform#41**.
- Live end-to-end acceptance test (`implement milestone mika#6`): **mika-platform#42**.
- Dashboard UI surfacing of `type`: separate ticket if/when needed (the field will be present in the API response so the UI can adopt it without re-shipping the agent).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs` — schema definition (clean-slate `migrate_v1`, line 841), migration ladder (line 700+), `Task`/`NewTask` structs (line 122+), `row_to_task` (line 2820), `TASK_COLUMNS` const (line 2854), `create_task` INSERT (line 2710), helper queries `find_active_work_item_by_ref_url` / `_by_label` / `_by_pr_url` / `_by_branch` (line 3120+). All hand-written column lists need updating in lockstep.
- `crates/mika-agent/src/db.rs` `migrate_v21_to_v22` (line 2504) — most recent migration; uses `ALTER TABLE … ADD COLUMN` guarded by `column_exists`. CHECK constraint on a new column requires the same guard pattern (SQLite supports `CHECK` on `ADD COLUMN` in 3.37+ which we ship).
- `crates/mika-agent/src/tools/create_work_item.rs` — input validation pattern (label, source enum, `validate_uuid`), dedup paths, audit logging, response formatting. New `type` parameter follows the existing source-enum validation pattern.
- `crates/mika-agent/src/tools/list_work_items.rs` — line-based output formatter with status/source/children fields. New `type` field appended to the per-item line.
- `crates/mika-agent/src/tools/check_work_item.rs` — `writeln!`-based output. New `Type:` line slots in alongside `Status:` / `Source:` / `Reference:`.
- `crates/mika-agent/src/server/dashboard.rs` `TaskResponse` (line 471) and `TaskDetailResponse` (line 586) — both DTOs need the `type` field for API consumers.

### Institutional Learnings

- `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — the "manual" trigger_type + "none" action_type is the existing work-item shape; new `type` column is orthogonal and slots cleanly.
- `docs/solutions/logic-errors/create-work-item-duplicate-on-retry.md` — dedup is a known sharp edge. New `type` column does not affect dedup keys (still `(agent_id, reference_url)` and `(agent_id, label COLLATE NOCASE)`); a milestone and an issue with the same `reference_url` would still dedup. This is acceptable: in practice the brainstorm shows milestone/project parents have distinct labels (`mika#6` for the milestone vs. `mika#581` for an issue), and a `reference_url` collision between a milestone and a child issue is not realistic.
- Schema-version pattern: every migration bumps `CURRENT_SCHEMA_VERSION`, adds itself to the migration ladder, updates the clean-slate `migrate_v1` baseline, and is logged in `MEMORY.md` + `crates/mika-agent/CLAUDE.md` + `docs/runtime-structure.md`.

### External References

None — straightforward SQLite ALTER TABLE + tool wiring.

## Key Technical Decisions

- **Rust field name uses `r#type`.** The SQL column, JSON serialization name, and tool parameter name are all `type` (per the issue and brainstorm). Rust requires `r#type` as a raw identifier. This keeps the external surface clean while paying the small Rust syntactic cost in the struct/SQL bindings.
- **Migration uses `ALTER TABLE … ADD COLUMN` with the CHECK constraint inline.** SQLite 3.37+ supports adding CHECK constraints via `ADD COLUMN`. We ship rusqlite-bundled (3.45+), so this is safe. No table rebuild required, which keeps the migration trivially fast on production DBs and avoids touching foreign keys.
- **Validate `type` at the tool boundary, not just at the DB CHECK.** The DB CHECK is defense-in-depth; the tool returns a structured human-readable error before INSERT to give the LLM an actionable message.
- **Dedup keys do not include `type`.** Adding `type` would let an agent accidentally create both `type=issue` and `type=milestone` rows pointing at the same `reference_url`. The brainstorm's mental model treats `reference_url` and `label` as identifying the *underlying object* — not its role in the work-item tree.
- **No new query helpers.** `list_work_items` already returns all manual tasks; `check_work_item` already loads a single task. The new `type` field flows through the existing `Task` struct without new SELECT shapes.
- **Schema bump is v22 → v23.** Aligns with the existing migration ladder.

## Open Questions

### Resolved During Planning

- **Should `type` be on `tasks` or only on manual work items?** On `tasks`. The CHECK constraint applies to all rows, and non-manual tasks (callback, recurring, a2a) all default to `'issue'` — they're not surfaced in `list_work_items`/`check_work_item` and the value is inert for them. Keeps the schema simple.
- **Should we add a partial index on `(agent_id, type)`?** No — the only foreseeable filter (`type='milestone'`) will hit a small subset and existing `idx_tasks_agent_status` already narrows result sets. Add later if mika-dev's milestone audits get slow.

### Deferred to Implementation

- Whether to expose `type` in the dashboard `TaskFilters` query string. Likely yes for parity, but this PR scopes to the response DTO only; query-string filtering can land in a follow-up if dashboard needs it.

## Implementation Units

- [ ] **Unit 1: Add `type` column via migration v22 → v23**

**Goal:** Schema and `Task`/`NewTask` Rust types carry the new `type` field. Existing data backfills to `'issue'` automatically.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
  - Bump `CURRENT_SCHEMA_VERSION` to 23.
  - Add `type` column with CHECK to clean-slate `migrate_v1` (the `CREATE TABLE tasks (…)` block at line 841).
  - Add `migrate_v22_to_v23` method using `ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'issue' CHECK (type IN ('issue', 'milestone', 'project'))`, guarded by `column_exists`.
  - Wire into the migration ladder (after the existing `migrate_v21_to_v22` block, line 746).
  - Add `r#type: String` to `Task` and `NewTask` structs (line 122-177).
  - Update `TASK_COLUMNS` const (line 2854) to include `type` as the final column.
  - Update `row_to_task` (line 2820) — bump `r.get(N)` ordinals; add `r#type` read at the new ordinal.
  - Bump `TASK_COLUMN_COUNT` (line 3058) from 29 → 30.
  - Update `create_task` INSERT (line 2710) — add `type` to the column list and bind `task.r#type`.
  - Update `create_recurring_task_if_absent` similarly (line 2755+).
  - Update `find_active_work_item_by_ref_url` and `find_active_work_item_by_label` (line 3123, 3230) — both use hand-written SELECT lists and inline row construction; add `type` to both.
  - Audit any other inline `SELECT` against `tasks` that reconstructs a `Task` struct outside `row_to_task` and add `type` there too.
- Test: `crates/mika-agent/src/db.rs` (existing inline `#[cfg(test)] mod tests` if present; otherwise covered by integration tests in Unit 4)

**Approach:**
- Migration order: 1) bump version constant, 2) add migration method, 3) wire into ladder, 4) update clean-slate `migrate_v1` schema. Doing them together ensures fresh DBs and migrated DBs end up with identical shape.
- The `type` column is appended at the end of the column list everywhere — last position in `TASK_COLUMNS`, last `r.get(…)` in `row_to_task`. This minimizes ordinal churn for existing reads but every hand-written SELECT/INSERT still has to be updated.

**Patterns to follow:**
- `migrate_v21_to_v22` (db.rs:2504) — idempotent `column_exists` guard, single `ALTER TABLE`, version insert.
- `messages.internal` field added in v22 (Schema v22 in `MEMORY.md`) — same pattern of "add column with NOT NULL DEFAULT, propagate through structs".

**Test scenarios:**
- Happy path: Fresh DB created via `migrate_v1` has the `type` column with DEFAULT `'issue'`. (Verify by inserting a task without `type` and reading back `type='issue'`.)
- Happy path: Migration on a v22 DB with existing rows leaves all rows with `type='issue'`.
- Edge case: Calling the migration twice (idempotency) does not error — `column_exists` guard short-circuits.
- Error path: Direct INSERT with `type='garbage'` fails with the SQLite CHECK constraint error.
- Integration: `create_task` round-trips `type='milestone'` correctly through `row_to_task`.

**Verification:**
- `cargo test -p mika-agent` passes.
- `sqlite3 mika.db "PRAGMA table_info(tasks)"` shows the `type` column with NOT NULL and DEFAULT `'issue'`.
- `sqlite3 mika.db "SELECT DISTINCT type FROM tasks"` returns `'issue'` only on a freshly migrated production DB.

- [ ] **Unit 2: Accept `type` parameter in `create_work_item`**

**Goal:** Tool accepts an optional `type` parameter, validates it, defaults to `'issue'`, and persists on the row.

**Requirements:** R4, R6

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/create_work_item.rs`
  - Add `VALID_TYPES: &[&str] = &["issue", "milestone", "project"]` constant near `VALID_SOURCES`.
  - Extend `input_schema` with a `type` property (enum of the three values, optional).
  - Parse `input["type"]` similar to `input["source"]`, default to `"issue"`, validate against `VALID_TYPES`, return structured error on invalid value.
  - Set `r#type` on the `NewTask` struct in both code paths (initial INSERT and the post-dedup-race retry block).
  - Include `Type: <value>` in the success response when type != `"issue"` (omit for default to keep messages compact for the common case).
- Test: `crates/mika-agent/src/tools/create_work_item.rs` (extend existing `#[cfg(test)] mod tests` block).

**Approach:**
- Validation runs after `label` validation, before guards, mirroring `source` validation.
- Default-to-issue handling lives in one place: a `let task_type = input["type"].as_str().unwrap_or("issue").trim()` line, then validate.
- Audit log `after_value` does not need to mention `type` — the row carries it; the response message is enough for the LLM to confirm.

**Patterns to follow:**
- `source` validation block (create_work_item.rs:107) — same shape: optional, enum-validated, persisted on `NewTask`.

**Test scenarios:**
- Happy path: No `type` parameter → row created with `type='issue'`, response omits the `Type:` line. (Verify via `ctx.db.get_task` or by extracting the new ID.)
- Happy path: `type='milestone'` → row created with `type='milestone'`, response includes `Type: milestone`.
- Happy path: `type='project'` → row created with `type='project'`, response includes `Type: project`.
- Happy path (explicit default): `type='issue'` → row created with `type='issue'`, response omits the `Type:` line (consistent with absent param).
- Error path: `type='epic'` → returns error with `Invalid type 'epic'. Must be one of: issue, milestone, project`, no row inserted (verify via row count).
- Error path: `type=''` (empty string) → treat as absent (defaults to `'issue'`); avoids gratuitous error on whitespace-only input.
- Integration: A `type='milestone'` row created with a `parent_task_id` correctly nests under the parent (existing depth/parent guards still apply).

**Verification:**
- `cargo test -p mika-agent --lib tools::create_work_item` passes.
- All four issue acceptance cases (no type, three valid, invalid) covered by named tests.

- [ ] **Unit 3: Surface `type` in `list_work_items` and `check_work_item`**

**Goal:** Both query tools include `type` in their output so agents can distinguish milestones from issues.

**Requirements:** R5, R6

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/list_work_items.rs`
  - In the per-item line formatter, append `type:<value>` to the trailing parenthetical (alongside `created:`, `ref:`, `src:`, `children:`). Omit when `type='issue'` (the default) to keep output compact for the common case.
- Modify: `crates/mika-agent/src/tools/check_work_item.rs`
  - Insert `Type: <value>` line right after `Status:`, before `Source:`. Always emit (since this tool inspects a single item, the explicit field aids clarity).
- Test: extend `#[cfg(test)] mod tests` blocks in both files.

**Approach:**
- `list_work_items`: hide `type:issue` from the listing (matches the pattern of hiding empty `ref_url`/`source`). Show `type:milestone` and `type:project` to make non-default rows visually distinct in long listings.
- `check_work_item`: always show `Type:` for unambiguous single-item inspection.

**Patterns to follow:**
- Existing trailing-field assembly in `list_work_items.rs:130-153` (the `ref_url`, `src`, `children` formatting).
- Existing `writeln!(output, "Status: …")` chain in `check_work_item.rs:222-244`.

**Test scenarios:**
- `list_work_items`: Items with default `type='issue'` produce no `type:` segment in the output (regression check on existing tests).
- `list_work_items`: A `type='milestone'` row produces `type:milestone` in its line.
- `list_work_items`: Mixed list (issue + milestone + project) renders all three correctly.
- `check_work_item`: Default `type='issue'` row shows `Type: issue`.
- `check_work_item`: `type='milestone'` row shows `Type: milestone`.

**Verification:**
- `cargo test -p mika-agent --lib tools::list_work_items` passes (existing + new tests).
- `cargo test -p mika-agent --lib tools::check_work_item` passes (existing + new tests).

- [ ] **Unit 4: Propagate `type` through dashboard DTOs**

**Goal:** `TaskResponse` and `TaskDetailResponse` include the `type` field so the dashboard API exposes it without breaking existing clients.

**Requirements:** R3 (carrying through to API surface)

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/server/dashboard.rs`
  - Add `pub r#type: String` (or `#[serde(rename = "type")] pub r#type: String`) to `TaskResponse` (line 471) and `TaskDetailResponse` (line 586). With raw identifiers, serde's default name is `"type"` — no rename needed; verify this in tests if uncertain.
  - Update the two `From<Task>` impls (line 496, 611) to copy `t.r#type`.
- Modify: `docs/openapi/mika-server.yaml` (and the build-time crate copy via `scripts/sync-agent-docs.sh`) — regenerate or hand-update the `TaskResponse` / `TaskDetailResponse` schemas to add the `type` property.
- Test: covered by `cargo test -p mika-agent` build + serde derive — no separate test file, but verify a snapshot test exists for the API or add a smoke test that serializes a `TaskResponse` and asserts `"type"` appears in the JSON.

**Approach:**
- Two-line change per DTO (add field, copy in `From`).
- OpenAPI doc regen happens in `/mika-doc-audit` step of the pipeline, but list it explicitly here so the implementer doesn't forget.

**Patterns to follow:**
- Existing optional fields (e.g. `source`, `reference_url`) in both DTOs.

**Test scenarios:**
- Happy path: Serializing a `TaskResponse` with `r#type = "milestone"` produces a JSON object containing `"type":"milestone"` (asserted via `serde_json::to_value`).
- Happy path: Default-typed task (`r#type = "issue"`) serializes with `"type":"issue"`.

**Verification:**
- `cargo build -p mika-agent` succeeds.
- `cargo test -p mika-agent` passes.

- [ ] **Unit 5: Documentation updates**

**Goal:** Schema docs and per-crate CLAUDE.md reflect v23 and the new `type` column. `MEMORY.md` records the migration.

**Requirements:** R1, R2 (documentation aspect)

**Dependencies:** Units 1-4

**Files:**
- Modify: `docs/runtime-structure.md` — add v22→v23 line to the migration history table (mirroring the v21→v22 entry); update the schema description for `tasks`.
- Modify: `crates/mika-agent/CLAUDE.md` — bump "Schema Version: v22" → "v23"; add a v22→v23 bullet under "Recent migrations".
- Modify: `~/.claude/projects/-data-workspace-mika-platform-mika/memory/MEMORY.md` — add v22→v23 entry under Schema Evolution; update "Current: Schema v22" → "v23".
- Run: `bash scripts/sync-agent-docs.sh` to copy `docs/runtime-structure.md` into the crate-local copy at `crates/mika-agent/docs/`. CI's `docs-sync` job enforces this.

**Approach:**
- Pure prose; no behavioral risk.
- The doc-audit pipeline step (`/mika-doc-audit`) will catch additional drift, but capturing the schema-version bump and the new column in this unit prevents `cargo build` ↔ docs lag during review.

**Patterns to follow:**
- v21→v22 entry in `MEMORY.md` and `crates/mika-agent/CLAUDE.md`.

**Test scenarios:**
- Test expectation: none — documentation unit, no behavioral change.

**Verification:**
- `bash scripts/sync-agent-docs.sh` produces no untracked diff in `crates/mika-agent/docs/`.
- CI `docs-sync` job passes locally (or is anticipated to pass on the PR).

## System-Wide Impact

- **Interaction graph:**
  - `tasks` table: column added — every read path that returns a `Task` struct must read the new column. Inline SELECT lists in `find_active_work_item_*` are the most fragile spots; the centralized `TASK_COLUMNS` const + `row_to_task` covers the rest.
  - `create_work_item` tool: parameter added — agents that don't pass `type` get `'issue'` (existing behavior).
  - Dashboard API: field added — additive change; existing clients ignore unknown fields.
- **Error propagation:** Validation error for invalid `type` returns `ToolOutput::error` (caught by the agent loop, surfaced to the LLM as a tool-call failure). DB CHECK constraint violation would surface as `anyhow::Error` from `create_task` — should never trigger in practice because tool-level validation catches it first.
- **State lifecycle risks:** None. The new column is set once at INSERT and never mutated by the existing flow. self-dev (separate ticket) may eventually update `type` via direct SQL or a new tool; that's out of scope here.
- **API surface parity:** Tool definition (`input_schema`) and response message both gain `type`. Dashboard DTO gains `type`. CLI commands like `mika tasks list` are unaffected (they don't filter or display type yet).
- **Integration coverage:**
  - End-to-end: create a milestone work item via `create_work_item type=milestone`, fetch via `check_work_item`, list via `list_work_items`, and confirm `type` round-trips correctly. Cover at least one integration test.
  - Migration on a populated v22 DB: existing rows backfill to `'issue'`. Add a focused test if not already covered by the migration test pattern in db.rs.
- **Unchanged invariants:**
  - `validate_dispatch_readiness` does not change behavior — `type` is not a dispatch-readiness signal.
  - Dedup behavior on `(agent_id, reference_url)` and `(agent_id, label)` is unchanged — `type` is not part of the dedup key.
  - `list_work_items` filters by `status` and `source` — no new `type` filter (deferred).
  - All existing `Task` consumers (CLI, TUI, server handlers) continue to work; they simply ignore the new field.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| A hand-written `SELECT … FROM tasks` reconstructs a `Task` struct without using `TASK_COLUMNS`/`row_to_task` and silently drops the new column. | Grep for `Task \{` and inline `SELECT` patterns over `tasks` during Unit 1 (`find_active_work_item_by_ref_url`, `_by_label`, `_by_pr_url`, `_by_branch` are known sites). The compiler will catch missing struct fields once `Task` is updated, since Rust requires all fields in struct literals — this risk is structurally bounded. |
| `r#type` ergonomics confuse contributors (`task.r#type` reads awkwardly). | Acceptable: every reference is one symbol; the SQL/JSON/tool name is the right user-facing surface. Document the choice in the Key Technical Decisions section. |
| Migration runs on a DB where someone has manually inserted `type='other'` (theoretical only — no path to do this today). | The migration adds a CHECK constraint via `ALTER TABLE ADD COLUMN`; SQLite enforces CHECK on subsequent writes only. A pre-existing invalid value would persist but is not reachable via any code path. Acceptable. |
| Dashboard clients that strictly validate API response shapes break on the added field. | Additive field on a response DTO is safe; serde clients ignore unknown fields. The dashboard at `dashboard/src/api/tasks.ts` uses TypeScript interfaces — adding a property to the API response does not break consumers that don't reference it. |

## Documentation / Operational Notes

- Migration runs automatically on agent startup. Zero-touch for operators.
- The `bash scripts/sync-agent-docs.sh` step (Unit 5) is mandatory for CI green; the `docs-sync` job in `.github/workflows/ci.yml` enforces it.
- No env vars added.
- No rollout coordination needed — additive schema change with sane default.

## Sources & References

- **Origin document:** [`mika-platform/docs/brainstorms/2026-04-15-milestones-and-projects-as-sprints-brainstorm.md`](../../mika-platform/docs/brainstorms/2026-04-15-milestones-and-projects-as-sprints-brainstorm.md)
- **GitHub issue:** senara-solutions/mika#595
- **Related code:**
  - `crates/mika-agent/src/db.rs` — schema, `Task`/`NewTask`, migrations
  - `crates/mika-agent/src/tools/create_work_item.rs` — tool implementation
  - `crates/mika-agent/src/tools/list_work_items.rs` — listing query
  - `crates/mika-agent/src/tools/check_work_item.rs` — single-item query
  - `crates/mika-agent/src/server/dashboard.rs` — `TaskResponse` / `TaskDetailResponse`
- **Related issues / PRs:**
  - mika-skills#149 — self-dev milestone/project workflow (depends on this)
  - mika-platform#41 — retire `/mika-sprint`, rename audit (depends on this + #149)
  - mika-platform#42 — live acceptance test (depends on all three)
- **Related learnings:**
  - `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md`
  - `docs/solutions/logic-errors/create-work-item-duplicate-on-retry.md`
