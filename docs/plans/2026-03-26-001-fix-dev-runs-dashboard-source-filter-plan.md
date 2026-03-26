---
title: "fix: dev runs dashboard filters by source='self_dev', excluding github_issue-triggered runs"
type: fix
status: completed
date: 2026-03-26
issue: "#277"
---

# fix: dev runs dashboard filters by source='self_dev', excluding github_issue-triggered runs

## Overview

The Dev Runs dashboard page (`/dashboard/dev-runs`) is invisible to dev runs triggered from GitHub issues because the SQL queries hardcode `source = 'self_dev'`. Tasks with `source = 'github_issue'` that have `run_claude_pilot` callback children are valid dev runs but are excluded from both the list and detail views.

## Root Cause

Two functions in `crates/mika-agent/src/db.rs` (~line 6340-6409) filter on `source = 'self_dev'`:

1. **`get_dev_run()`** (line 6354) — single dev run lookup
2. **`list_dev_runs_paginated_with_count()`** (line 6366) — paginated list with count, 4 SQL strings

The `source` field indicates how the task was created (self-dev skill, GitHub issue, etc.), not whether it's a dev run. A dev run is any work item where mika-dev launches claude-pilot, regardless of originating source.

## Fix

Change `source = 'self_dev'` to `source IN ('self_dev', 'github_issue')` in all affected SQL queries. Update associated comments and doc comments.

### Also add `source` to API response

Add the `source` field to `DevRunResponse` and the frontend `DevRun` TypeScript interface so users can distinguish the origin of each dev run. Display a small badge in the table ("Self Dev" vs "Issue").

## Acceptance Criteria

- [x] `get_dev_run()` returns dev runs with `source = 'github_issue'` — `crates/mika-agent/src/db.rs`
- [x] `list_dev_runs_paginated_with_count()` lists dev runs with `source = 'github_issue'` — `crates/mika-agent/src/db.rs`
- [x] Count query matches data query (both use updated filter) — `crates/mika-agent/src/db.rs`
- [x] Section header comment updated to reflect both sources — `crates/mika-agent/src/db.rs:6340`
- [x] Doc comments on both functions updated — `crates/mika-agent/src/db.rs`
- [x] `DevRunResponse` includes `source: Option<String>` — `crates/mika-agent/src/server/dashboard_dev_runs.rs`
- [x] Frontend `DevRun` type includes `source` field — `dashboard/src/api/devRuns.ts`
- [x] Dev runs table shows source badge — `dashboard/src/pages/DevRuns.tsx`
- [x] `cargo test` passes
- [x] `cargo clippy` passes
- [x] Dashboard builds (`npm run build --prefix dashboard`)

## Files to Change

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Update SQL in `get_dev_run()` and `list_dev_runs_paginated_with_count()`: `source = 'self_dev'` → `source IN ('self_dev', 'github_issue')`. Update section header and doc comments. |
| `crates/mika-agent/src/server/dashboard_dev_runs.rs` | Add `source: Option<String>` to `DevRunResponse` struct, populate from query result. |
| `dashboard/src/api/devRuns.ts` | Add `source?: string` to `DevRun` interface. |
| `dashboard/src/pages/DevRuns.tsx` | Display source badge in the table (e.g., "Issue" / "Self Dev"). |

## Files NOT Changed

| File | Reason |
|------|--------|
| `crates/mika-agent/src/async_db.rs` | Thin wrapper, delegates to `db.rs`, no SQL |
| `dashboard/src/pages/DevRunDetail.tsx` | Detail view already works via `get_dev_run()` fix |
| `crates/mika-agent/src/tools/create_work_item.rs` | Source validation is correct, no changes needed |

## Sources & References

- GitHub Issue: #277
- Learning: `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — `source` is free-text, no CHECK constraint
- Learning: `docs/solutions/architecture-patterns/delegate-channel-type-taxonomy.md` — analogous bug pattern (overly restrictive filter)
