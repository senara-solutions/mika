---
title: "feat: Claude-pilot dashboard integration — structured run metadata + Dev Runs page"
type: feat
status: in-progress
date: 2026-03-22
issue: "#236"
origin: docs/brainstorms/2026-03-22-claude-pilot-dashboard-integration-brainstorm.md
---

# feat: Claude-pilot dashboard integration — structured run metadata + Dev Runs page

## Overview

Make the autonomous dev loop (mika-dev → claude-pilot → Claude Code → PR) observable and actionable through the Mika dashboard. This PR delivers the MVP: structured metadata on work items, a dashboard Dev Runs page to view them, and a merge action to close the loop.

## Problem Statement / Motivation

Dev runs are invisible today — scattered across Telegram notifications, raw log files, and GitHub. The data partially exists (work items in `tasks` table, GitHub PRs) but there's no structured way to correlate a dev run's branch, PR, cost, duration, and turns. The dashboard has pages for everything except dev work.

(see brainstorm: docs/brainstorms/2026-03-22-claude-pilot-dashboard-integration-brainstorm.md)

## Scope — This PR

**In scope:**
1. Schema v14: add `metadata TEXT` column to `tasks` table
2. Rust `Task`/`NewTask` structs gain `metadata` field
3. `update_work_item_status` tool accepts optional `metadata` JSON
4. New `update_work_item_metadata` DB function (merge-update, not replace)
5. Dashboard API: `GET /api/v1/dev-runs` (paginated, filtered work items with `source='self_dev'`)
6. Dashboard API: `GET /api/v1/dev-runs/{task_id}` (single dev run detail)
7. Dashboard API: `POST /api/v1/dev-runs/{task_id}/merge` (merge PR via GitHub CLI)
8. Dashboard frontend: Dev Runs list page + detail page + merge button
9. Sidebar nav entry for Dev Runs

**Deferred (separate issues):**
- mika-dev GitHub account for autonomous merges (account setup, merge policy)
- claude-pilot structured callback format (claude-pilot repo issue)
- Live streaming logs via WebSocket
- Sprint view grouping

## Technical Approach

### 1. Schema v14 — add `metadata` column

Simple `ALTER TABLE` migration (no full table rebuild needed since SQLite supports adding nullable columns).

```sql
ALTER TABLE tasks ADD COLUMN metadata TEXT;
```

The `metadata` column stores a JSON string. For dev runs (work items with `source='self_dev'`), the expected shape is:

```json
{
  "claude_pilot": {
    "session_id": "5168bdf9",
    "branch": "feat/19/ecr-iam",
    "worktree_path": ".claude/worktrees/feat-19-ecr-iam/mika-cloud",
    "pr_number": 43,
    "pr_url": "https://github.com/senara-solutions/mika-cloud/pull/43",
    "repo": "mika-cloud",
    "cost_usd": 1.50,
    "duration_ms": 87000,
    "turns": 9,
    "log_path": "/var/log/claude-pilot/14a48ba6.log"
  }
}
```

No enum or CHECK constraint — JSON is opaque to SQLite, validated in Rust.

### 2. Rust struct changes

`Task` and `NewTask` both gain `pub metadata: Option<String>`. All `SELECT` and `INSERT` queries that touch the `tasks` table must include the new column. The `From<Task>` for `TaskResponse` (in dashboard.rs) passes through metadata as-is.

### 3. `update_work_item_status` tool — accept metadata

Add an optional `metadata` parameter to the tool's JSON schema. When provided, it's validated as a JSON object (not array, not scalar) and merged into the existing metadata using a shallow merge of the top-level keys. This lets mika-dev populate metadata incrementally:

1. At launch: `{ "claude_pilot": { "branch": "...", "repo": "..." } }`
2. After callback: merge in `{ "claude_pilot": { "session_id": "...", "cost_usd": ... } }`
3. After PR creation: merge in `{ "claude_pilot": { "pr_number": ..., "pr_url": "..." } }`

The merge happens at the `claude_pilot` key level — the nested object is replaced, not deep-merged. This keeps the logic simple. The tool calls `update_work_item_metadata` on the DB.

### 4. Dashboard API endpoints

Three new endpoints, all behind `dashboard_or_internal_token` auth:

**`GET /api/v1/dev-runs`** — Paginated list of work items filtered to `source='self_dev'`.
- Query params: `status`, `page`, `per_page`
- Returns `PaginatedResponse<DevRunResponse>`
- `DevRunResponse` is a projection of `Task` with parsed metadata fields surfaced as top-level:
  ```rust
  #[derive(Debug, Serialize)]
  pub struct DevRunResponse {
      pub id: String,
      pub agent_id: String,
      pub label: String,
      pub status: String,
      pub reference_url: Option<String>,
      pub created_at: String,
      pub updated_at: String,
      pub completed_at: Option<String>,
      // Extracted from metadata.claude_pilot:
      pub branch: Option<String>,
      pub repo: Option<String>,
      pub pr_number: Option<u32>,
      pub pr_url: Option<String>,
      pub cost_usd: Option<f64>,
      pub duration_ms: Option<u64>,
      pub turns: Option<u32>,
      pub session_id: Option<String>,
  }
  ```

**`GET /api/v1/dev-runs/{task_id}`** — Single dev run detail. Same `DevRunResponse` shape. Returns 404 if not found or not a `source='self_dev'` work item.

**`POST /api/v1/dev-runs/{task_id}/merge`** — Trigger a PR merge.
- Reads `pr_url` from the work item's metadata
- Extracts `owner/repo` and PR number from the URL
- Shells out to `gh pr merge <number> --repo <owner/repo> --merge --delete-branch`
- Requires `MIKA_INVESTIGATE_GITHUB_TOKEN` (reuses existing env var)
- Updates work item status to `completed` on success
- Returns `{ "merged": true, "pr_url": "..." }` or error

### 5. DB layer additions

New functions in `db.rs` / `async_db.rs`:

- `update_work_item_metadata(task_id: &str, metadata_json: &str) -> Result<bool>` — Updates the `metadata` column. Only works on `trigger_type='manual'` tasks. Returns false if not found.
- `list_dev_runs_paginated_with_count(filters: DevRunFilters, per_page: u32, offset: u32) -> Result<(Vec<Task>, u64)>` — Queries tasks WHERE `trigger_type='manual' AND source='self_dev'` with optional status filter, ordered by `created_at DESC`.
- `get_dev_run(task_id: &str) -> Result<Option<Task>>` — Gets a single task WHERE `trigger_type='manual' AND source='self_dev'` (unscoped by agent_id, for dashboard).

### 6. Dashboard frontend

**New files:**
- `dashboard/src/pages/DevRuns.tsx` — List page (mirrors TeamRuns.tsx pattern)
- `dashboard/src/pages/DevRunDetail.tsx` — Detail page with metadata display + merge button
- `dashboard/src/api/devRuns.ts` — API hooks and types

**Modified files:**
- `dashboard/src/App.tsx` — Add routes for `/dev-runs` and `/dev-runs/:taskId`
- `dashboard/src/components/Sidebar.tsx` — Add "Dev Runs" nav item with `Hammer` icon from lucide-react

The Dev Runs list page shows a table with columns:

| Column | Source | Notes |
|--------|--------|-------|
| Issue | `reference_url` | Linked to GitHub issue |
| Branch | `metadata.claude_pilot.branch` | Short display |
| Status | `status` | Badge (pending/in_progress/completed/failed) |
| PR | `metadata.claude_pilot.pr_url` | Linked to GitHub PR |
| Cost | `metadata.claude_pilot.cost_usd` | e.g., "$1.50" |
| Duration | `metadata.claude_pilot.duration_ms` | e.g., "1m 27s" |
| Created | `created_at` | Relative time |

Filter bar: status dropdown only (all dev runs share `source='self_dev'`).

The detail page shows all metadata fields plus a "Merge PR" button (enabled only when `pr_url` exists and status is `in_progress`). Merge button shows confirmation dialog, calls POST endpoint, and refreshes on success.

## File-by-File Changes

### Rust — Schema & DB

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `metadata: Option<String>` to `Task` and `NewTask` structs. Add `metadata` column to v1 CREATE TABLE (clean-slate). Add `migrate_v13_to_v14()` with ALTER TABLE. Update `CURRENT_SCHEMA_VERSION` to 14. Add `update_work_item_metadata()`, `list_dev_runs_paginated_with_count()`, `get_dev_run()` functions. Update all `SELECT`/`INSERT` queries on tasks to include `metadata` column. |
| `crates/mika-agent/src/async_db.rs` | Add async wrappers: `update_work_item_metadata()`, `list_dev_runs_paginated_with_count()`, `get_dev_run()`. |

### Rust — Tool

| File | Change |
|------|--------|
| `crates/mika-agent/src/tools/update_work_item_status.rs` | Add optional `metadata` parameter to `definition()` JSON schema. In `execute()`, validate metadata is a JSON object, call `update_work_item_metadata()` when provided. Update tests. |

### Rust — Server / Dashboard API

| File | Change |
|------|--------|
| `crates/mika-agent/src/server/mod.rs` | Register 3 new routes under dashboard_routes: `GET /dev-runs`, `GET /dev-runs/{task_id}`, `POST /dev-runs/{task_id}/merge`. |
| `crates/mika-agent/src/server/dashboard.rs` | Add `DevRunResponse` struct, `DevRunsQuery` struct, `handle_dev_runs_list()`, `handle_dev_run_detail()`, `handle_dev_run_merge()` handlers. Add `DevRunFilters` to db imports. |

### TypeScript — Dashboard Frontend

| File | Change |
|------|--------|
| `dashboard/src/api/devRuns.ts` | **New file.** `DevRun` interface, `DevRunsFilters` interface, `useDevRuns()`, `useDevRun()` hooks, `mergeDevRunPR()` mutation function. |
| `dashboard/src/pages/DevRuns.tsx` | **New file.** Paginated list page with status filter, table matching columns spec above. Mirrors TeamRuns.tsx pattern. |
| `dashboard/src/pages/DevRunDetail.tsx` | **New file.** Detail page showing all metadata fields, merge button with confirmation dialog. |
| `dashboard/src/App.tsx` | Import DevRuns/DevRunDetail, add two Route entries. |
| `dashboard/src/components/Sidebar.tsx` | Add `{ to: '/dev-runs', label: 'Dev Runs', icon: Hammer }` to navItems. Import `Hammer` from lucide-react. |

### Documentation

| File | Change |
|------|--------|
| `docs/runtime-structure.md` | Add `metadata TEXT` to tasks table schema reference. Document schema v14. |

## Acceptance Criteria

1. **Schema v14 migration runs cleanly** — `cargo test` passes, existing work items retain all data, new `metadata` column is NULL by default
2. **`update_work_item_status` accepts metadata** — tool validates JSON object shape, merges into existing metadata, rejects non-object JSON
3. **`GET /api/v1/dev-runs`** — returns paginated work items filtered to `source='self_dev'`, status filter works, metadata fields extracted into response
4. **`GET /api/v1/dev-runs/{task_id}`** — returns single dev run with parsed metadata, 404 for non-dev-run tasks
5. **`POST /api/v1/dev-runs/{task_id}/merge`** — merges PR via `gh`, updates status, returns success/error, requires GitHub token
6. **Dashboard Dev Runs page** — renders table with all specified columns, status filter works, pagination works, empty state shown when no dev runs
7. **Dashboard Dev Run detail** — shows full metadata, merge button visible when PR exists, confirmation dialog before merge, success/error feedback
8. **Sidebar** — "Dev Runs" entry appears between Tasks and Team Runs
9. **All existing tests pass** — `cargo test`, `cargo clippy` clean
10. **No new external crates** — uses existing dependencies only

## Work Log

| Step | Status | Notes |
|------|--------|-------|
| Write plan | ✅ | This document |
| Schema v14 + DB layer | | |
| update_work_item_status metadata | | |
| Dashboard API endpoints | | |
| Dashboard frontend (DevRuns + DevRunDetail) | | |
| Sidebar + routing | | |
| Tests + clippy | | |
| Doc updates | | |
