---
title: "feat: Add task tree visualization to Dev Runs detail page"
type: feat
status: active
date: 2026-05-05
issue: 661
---

# feat: Add task tree visualization to Dev Runs detail page

## Overview

Replace the flat "Child Tasks" list on the Dev Runs detail page with a collapsible task tree that reveals the full hierarchy of descendants beneath the root dev run task. The root task itself is already rendered as the page header (title, status badge, stats row); the tree visualizes everything below it: Issue → long_running callback → downstream tasks. Add a recursive descendants backend endpoint (bounded by the existing depth 0-3 CHECK constraint) and a dashboard-local `<TaskTree />` component.

## Problem Frame

Dev Runs have a tree-shaped execution structure (milestone → issue → run_claude_pilot callback → build/deploy tasks), but the detail page renders children as a flat list using `GET /api/v1/tasks/:id/children` (direct children only). Operators must mentally reconstruct the hierarchy by cross-referencing the Child Tasks, Agent Activity, and Claude Pilot Metadata sections. This makes it hard to trace failures through the dispatch chain.

## Requirements Trace

- R1. Dev Runs detail page renders child tasks as a collapsible tree with expand/collapse
- R2. Tree shows the full lineage: milestone → issue → long_running → callbacks → build/deploy
- R3. Each tree node shows: label, status badge (via `<TaskStatusBadge />`), trigger_type, timing, link to detail
- R4. Top-level children expanded by default; deeper levels collapsed
- R5. Agent Activity sessions section covers the full task tree (all descendants), not just root + direct children

## Scope Boundaries

- No real-time WebSocket push — polling via `refetchInterval` when the tree has non-terminal tasks
- No drag-and-drop or tree manipulation — read-only visualization
- No generic `<TreeView />` in `@senara-solutions/ui` — the tree component starts as a dashboard-local component per the stitch-map workflow agreement ("primitives used by only one surface stay local"); promote when Cloud Console or another surface needs it

### Deferred to Separate Tasks

- `<TaskTree />` extraction to `@senara-solutions/ui`: separate PR when a second consumer surfaces
- Back-navigation context preservation (e.g., `?from=/dev-runs/:id` on task links): general dashboard UX improvement, not specific to this tree
- Team Runs detail tree adoption: follow-up ticket after #652 lands

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs:5084` — `get_child_tasks()`: direct children only, `WHERE parent_task_id = ?1`, no agent_id filter
- `crates/mika-agent/src/db.rs:8087` — `get_sessions_for_task_tree()`: collects root + direct children task IDs only
- `crates/mika-agent/src/server/dashboard.rs:728` — `handle_task_children()` handler
- `crates/mika-agent/src/server/mod.rs:126` — route registration for `/tasks/{task_id}/children`
- `dashboard/src/pages/DevRunDetail.tsx:513-554` — current flat Child Tasks section
- `dashboard/src/api/tasks.ts:84-90` — `useTaskChildren()` hook
- `dashboard/src/components/CollapsibleCard.tsx` — expand/collapse pattern used throughout
- `packages/ui/src/components/ListRow.tsx` — `<ListRow variant="expandable">` with chevron, keyboard a11y, `isTargetRow()` guard
- Tasks DB: `depth` column has `CHECK (depth BETWEEN 0 AND 3)` — max 4 levels, recursion is inherently bounded

### Institutional Learnings

- `docs/solutions/dashboard-issues/task-session-bidirectional-linking.md` — `get_sessions_for_task_tree()` is shallow (root + direct children only); needs extending for full tree
- `docs/solutions/best-practices/design-system-listrow-extraction-2026-04-27.md` — `<ListRow variant="expandable">` auto-injects chevron; static rows need `<td className="w-8" />` spacer for alignment
- `docs/solutions/best-practices/design-system-status-pill-migration-2026-04-27.md` — hand-rolled status pills are review fails; use `<TaskStatusBadge />`
- `docs/solutions/best-practices/design-system-state-catalog-extraction-2026-04-27.md` — use `<LoadingState />`, `<ErrorState />`, `<EmptyState />` from `@senara-solutions/ui`
- `docs/solutions/dashboard-issues/dev-runs-source-filter-too-restrictive.md` — descendants query must not filter by single source
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — use centralized column constants for SQL queries

### Design References

- Stitch screen #8 (`abe0ec4a059d459f94220fad9404149a`) — Team Run Debug Detail: "The Iteration Timeline IS the answer to #661 task-tree visualization"
- Stitch variant `e7cd46efd53b4c0d91e689edff4fa877` — expanded-node detail rendering (editorial typography, prominent labels)

## Key Technical Decisions

- **Recursive CTE with depth guard:** SQLite `WITH RECURSIVE` is safe here — max depth is 3 (DB-enforced), so worst-case recursion is 4 levels. Include `WHERE t.depth <= 3` in the CTE as defense-in-depth against any data anomalies. No row LIMIT — the depth constraint naturally bounds results (typical tree: ~90 nodes for a 30-issue milestone; depth 0-3 makes unbounded growth impossible).
- **Root task excluded from descendants:** The root dev run task is already rendered as the page header (title, status badge, stats). The descendants endpoint returns only the subtree below it. The `<TaskTree />` component renders these descendants with indentation relative to the root, not using absolute DB `depth` values.
- **Depth is absolute in DB, relative in rendering:** The `depth` column stores absolute tree depth (0-3, set at creation as `parent.depth + 1`). The tree component computes visual indentation as `node.depth - minDepthInResults` to handle cases where the root is not at depth 0.
- **Single endpoint, flat response with parent_task_id:** Return all descendants as a flat array with `parent_task_id` references. Frontend builds the tree client-side using a simple `Map<parentId, children[]>` grouping. This is simpler than returning a nested JSON tree and reuses the existing `TaskResponse` DTO.
- **Replace `useTaskChildren` with `useTaskDescendants` on DevRunDetail:** The descendants response is a superset of children. No need to call both endpoints.
- **Update `get_sessions_for_task_tree` to use all descendant IDs:** Extends the ID collection to include all descendants. The root task's own sessions are still included (root ID added first, then descendant IDs appended — same pattern as current code, just with more IDs). Ensures the Agent Activity section shows sessions for the entire tree.
- **Dashboard-local tree component:** Per stitch-map workflow agreement, start local in `dashboard/src/components/TaskTree.tsx`. No `@senara-solutions/ui` extraction until a second consumer surfaces.
- **Conditional polling:** `useTaskDescendants` enables `refetchInterval: 15000` when any descendant has a non-terminal status. Stops when all tasks reach terminal state.

## Open Questions

### Resolved During Planning

- **Row limit on descendants?** No LIMIT needed. The depth 0-3 CHECK constraint bounds the tree at 4 levels; a large milestone produces ~90 nodes. Depth constraint makes unbounded growth structurally impossible.
- **Should sessions query also go recursive?** Yes — `get_sessions_for_task_tree()` already collects task IDs and queries sessions; we extend the ID collection to include all descendants. Root task sessions are still included.
- **Default expand state for deeper levels?** Direct children of the root task expanded by default. Deeper descendants collapsed. Expand logic uses relative depth (distance from root in tree), not absolute DB `depth` column, to handle cases where the root is not at depth 0.

### Deferred to Implementation

- Exact CSS indentation per depth level — implement and tune visually
- Whether trigger_type badges need different colors — use existing badge chip styling first, adjust if unclear

## Implementation Units

- [x] **Unit 1: Backend — recursive descendants query**

  **Goal:** Add `get_task_descendants()` to the DB layer that returns all descendants of a task using a recursive CTE.

  **Requirements:** R1, R2

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs`
  - Modify: `crates/mika-agent/src/async_db.rs`
  - Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - Add `get_task_descendants(root_task_id: &str) -> Result<Vec<Task>>` using `WITH RECURSIVE` CTE
  - CTE anchor: `SELECT ... FROM tasks WHERE parent_task_id = ?1` (direct children of root, not root itself)
  - Recursive step: `UNION ALL SELECT t.* FROM tasks t JOIN descendants d ON t.parent_task_id = d.id WHERE t.depth <= 3`
  - No row LIMIT — depth CHECK (0-3) structurally bounds the result set
  - Reuse `Self::TASK_COLUMNS` and `Self::row_to_task` — same pattern as `get_child_tasks()`
  - No agent_id filter (same as `get_child_tasks` — team trees have cross-agent children)
  - Add async wrapper `AsyncDatabase::get_task_descendants()`

  **Patterns to follow:**
  - `get_child_tasks()` at db.rs:5084 — same column set, same row mapper, same no-agent-id design
  - `AsyncDatabase` closure-based dispatch pattern

  **Test scenarios:**
  - Happy path: create a 3-level tree (root → child → grandchild → great-grandchild), verify all 3 descendants returned in creation order
  - Happy path: verify root task is NOT included in results (only descendants)
  - Edge case: task with no children returns empty vec
  - Edge case: task with children at only one level returns only direct children (same as `get_child_tasks`)
  - Edge case: multi-branch tree (root with 3 children, each with 2 grandchildren) returns all 9 descendants
  - Edge case: cross-agent children included (parent agent_id="a", child agent_id="b")

  **Verification:**
  - `cargo test -p mika-agent get_task_descendants` passes
  - Query returns descendants ordered by `created_at ASC`

- [x] **Unit 2: Backend — descendants API endpoint**

  **Goal:** Expose `GET /api/v1/tasks/:id/descendants` returning all descendants as a flat `TaskResponse[]`.

  **Requirements:** R1, R2

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/server/dashboard.rs`
  - Modify: `crates/mika-agent/src/server/mod.rs`

  **Approach:**
  - Add `handle_task_descendants()` handler following the exact pattern of `handle_task_children()` at dashboard.rs:728
  - Map `Vec<Task>` → `Vec<TaskResponse>` using existing `TaskResponse::from()`
  - Register route at `/tasks/{task_id}/descendants` in `mod.rs` alongside the existing `/tasks/{task_id}/children` route

  **Patterns to follow:**
  - `handle_task_children()` at dashboard.rs:728 — same State/Path extraction, same error handling, same `TaskResponse::from()` mapping

  **Test scenarios:**
  - Happy path: endpoint returns 200 with array of TaskResponse objects for a task with descendants
  - Edge case: endpoint returns 200 with empty array for a task with no descendants
  - Edge case: endpoint returns 200 with empty array for a non-existent task ID (consistent with `get_child_tasks` behavior)

  **Verification:**
  - Endpoint accessible via `curl http://localhost:8080/api/v1/tasks/<id>/descendants` with auth token
  - Response shape matches existing `GET /api/v1/tasks/:id/children`

- [x] **Unit 3: Backend — extend sessions-for-task-tree to include all descendants**

  **Goal:** Update `get_sessions_for_task_tree()` to collect sessions for the root task AND all its descendants (not just direct children).

  **Requirements:** R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs`
  - Test: `crates/mika-agent/src/db.rs` (inline tests)

  **Approach:**
  - Replace the direct-children-only task ID collection (db.rs:8088-8098) with a call to `get_task_descendants()` to get all descendant task IDs
  - Root task ID is still added first (same as current code at line 8089: `let mut task_ids = vec![root_task_id.to_string()]`), then all descendant IDs appended. Root's own sessions are preserved.
  - Rest of the function (session query with `COALESCE` for backward compat) stays unchanged

  **Patterns to follow:**
  - Current `get_sessions_for_task_tree()` at db.rs:8087 — preserve the session query structure and backward-compat COALESCE

  **Test scenarios:**
  - Happy path: sessions linked to grandchild tasks (depth 2) are now returned
  - Integration: existing test `test_get_sessions_for_task_tree` still passes (root + direct children sessions found)
  - Edge case: sessions linked to depth-3 tasks are found

  **Verification:**
  - Existing `test_get_sessions_for_task_tree` and `test_sessions_for_task_tree_backfill_compat` still pass
  - New test with deeper tree hierarchy confirms sessions at all levels are returned

- [x] **Unit 4: Frontend — `useTaskDescendants` hook**

  **Goal:** Add a TanStack React Query hook to fetch the full descendants tree from the new endpoint, with conditional polling.

  **Requirements:** R1, R2

  **Dependencies:** Unit 2

  **Files:**
  - Modify: `dashboard/src/api/tasks.ts`

  **Approach:**
  - Add `useTaskDescendants(taskId: string | undefined)` hook returning `TaskItem[]`
  - Follow `useTaskChildren` pattern exactly: `useQuery` with `enabled: !!taskId`, query key `['task-descendants', taskId]`
  - Add `refetchInterval` callback that checks response data: return `15_000` if any task has non-terminal status (`pending`, `in_progress`, `blocked`, `suspended`), return `false` otherwise

  **Patterns to follow:**
  - `useTaskChildren` at tasks.ts:84-90

  **Test expectation:** none — hook is a thin fetch wrapper with no logic beyond the polling gate

  **Verification:**
  - Hook returns the same shape as `useTaskChildren` (same `TaskItem` type)
  - Browser network tab confirms polling stops when all tasks are terminal

- [x] **Unit 5: Frontend — TaskTree component**

  **Goal:** Build a collapsible tree component that renders task descendants as an indented hierarchy with expand/collapse controls.

  **Requirements:** R1, R2, R3, R4

  **Dependencies:** Unit 4

  **Files:**
  - Create: `dashboard/src/components/TaskTree.tsx`

  **Approach:**
  - Props: `{ tasks: TaskItem[], rootTaskId: string }` — receives flat array, builds tree client-side
  - Build tree structure: `Map<string | null, TaskItem[]>` grouping by `parent_task_id`, then render recursively
  - Each node renders: indent per depth, chevron (if has children), `<TaskStatusBadge />`, label as `<Link to={/tasks/${id}}>`, trigger_type badge, timing (relative time from `created_at`), optional SESSION/TRACE links
  - Expand/collapse state: `Set<string>` of expanded node IDs. Initialize by expanding all direct children of `rootTaskId` (nodes whose `parent_task_id === rootTaskId`), deeper levels collapsed. Uses tree-structural relationship, not absolute DB `depth` column
  - Chevron uses `ChevronRight`/`ChevronDown` from lucide-react (same pattern as `SessionRow` in DevRunDetail.tsx)
  - Indentation via `paddingLeft` calculated from depth relative to root (e.g., `(node.depth - rootDepth) * 24px`)
  - Use `<div>` structure (not `<table>`) — the tree is not tabular data; the existing child tasks section uses divs too

  **Patterns to follow:**
  - `SessionRow` in DevRunDetail.tsx (lines 162-203) — expand/collapse with ChevronDown rotation animation
  - Child Tasks section (lines 521-553) — row layout with status badge, link, SESSION/TRACE chips, action_type badge
  - Dashboard card styling: `bg-bg-card border border-white/[0.05] rounded-2xl p-5`

  **Test expectation:** none — presentational React component with local state only; behavior verified via manual visual inspection and dashboard dev server

  **Verification:**
  - Component renders a multi-level tree with correct indentation
  - Clicking a chevron expands/collapses that node's children
  - Depth-1 nodes start expanded, deeper nodes start collapsed
  - Uses `<TaskStatusBadge />` for status (not hand-rolled pills)
  - Leaf nodes (no children) render without a chevron

- [x] **Unit 6: Frontend — integrate TaskTree into DevRunDetail**

  **Goal:** Replace the flat "Child Tasks" section with the new `<TaskTree />` component, wired to `useTaskDescendants`.

  **Requirements:** R1, R2, R3, R4, R5

  **Dependencies:** Unit 4, Unit 5

  **Files:**
  - Modify: `dashboard/src/pages/DevRunDetail.tsx`

  **Approach:**
  - Replace `useTaskChildren(taskId)` import and call with `useTaskDescendants(taskId)`
  - Replace the "Child Tasks" section (lines 513-554) with `<TaskTree tasks={descendants} rootTaskId={taskId} />`
  - Add loading/empty state handling: `<LoadingState variant="list" />` while descendants load, hide section when empty (same pattern as current)
  - Update section header from "Child Tasks (N)" to "Task Tree (N)" with descendant count
  - Remove `useTaskChildren` import (no longer used on this page)

  **Patterns to follow:**
  - Current "Agent Activity" section (lines 492-511) — conditional rendering when data exists, section header with count

  **Test scenarios:**
  - Happy path: DevRunDetail page renders TaskTree when descendants exist
  - Edge case: section hidden when no descendants
  - Edge case: loading state shown while descendants fetch is in flight
  - Integration: clicking a task link in the tree navigates to `/tasks/:id`

  **Verification:**
  - Dev Runs detail page shows a collapsible tree instead of a flat list
  - Tree matches the full hierarchy depth visible in the tasks DB
  - Agent Activity section now shows sessions for all descendants (via Unit 3)

## System-Wide Impact

- **Interaction graph:** The new descendants endpoint is consumed only by DevRunDetail. The existing children endpoint remains for TaskDetail and any other consumers.
- **Error propagation:** Recursive CTE errors surface as 500 via the standard `internal_error()` handler, rendered by `<ErrorState />` on the frontend.
- **State lifecycle risks:** No write operations. Read-only query with bounded recursion (depth 0-3 CHECK constraint + CTE depth guard + 200 row LIMIT).
- **API surface parity:** The children endpoint stays unchanged. The descendants endpoint is additive.
- **Integration coverage:** Sessions-for-task-tree change means the Agent Activity section will show more sessions than before — existing users may notice new sessions appearing that were previously hidden (deeper tree sessions). This is correct behavior.
- **Unchanged invariants:** `get_child_tasks()` and `handle_task_children()` are untouched. `TaskResponse` DTO is unchanged. The `useTaskChildren` hook remains available for other pages.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Recursive CTE performance on large trees | Bounded by depth CHECK (0-3); worst case is ~100 rows, trivially fast for SQLite |
| Circular parent_task_id references | CTE depth guard (`WHERE t.depth <= 3`) prevents runaway recursion; SQLite's default 1000-recursion limit is a second backstop |
| Breaking existing sessions behavior | Unit 3 extends the existing ID collection but preserves the session query structure; existing tests still pass |
| TaskTree component complexity | Component is div-based with local state only; no complex state management needed for 4-level bounded trees |

## Sources & References

- Related issues: #661 (this), #657 (StatusPill), #651 (Dev Runs detail), #652 (Team Runs detail), #13 (milestone)
- Design: Stitch screen #8 (`abe0ec4a059d459f94220fad9404149a`), variant `e7cd46efd53b4c0d91e689edff4fa877`
- Design docs: `docs/design/dashboard-stitch-map.md`
