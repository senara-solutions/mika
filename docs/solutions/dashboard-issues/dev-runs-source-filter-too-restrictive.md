---
title: "Dev runs dashboard filtered by source='self_dev', excluding github_issue-triggered runs"
category: dashboard-issues
date: 2026-03-26
tags: [dashboard, sql-filter, dev-runs, source-field, taxonomy]
severity: medium
modules: [mika-agent/db, mika-agent/server/dashboard_dev_runs, dashboard]
issue: "#277"
---

# Dev Runs Dashboard Source Filter Too Restrictive

## Problem

The Dev Runs dashboard page (`/dashboard/dev-runs`) was invisible to dev runs triggered from GitHub issues. Tasks with `source = 'github_issue'` that had `run_claude_pilot` callback children were valid dev runs but were excluded from both list and detail views.

**Symptom:** A task like `7f68673b-...` (mika-dev implementing mika-skills#30) with `source = 'github_issue'` and a `run_claude_pilot` callback child was a dev run but invisible in the dashboard.

## Root Cause

Two SQL query functions in `crates/mika-agent/src/db.rs` hardcoded `source = 'self_dev'`:

- `get_dev_run()` — single dev run lookup
- `list_dev_runs_paginated_with_count()` — paginated list (4 SQL strings)

The `source` field indicates *how* the task was created (self-dev skill, GitHub issue, etc.), not *whether* it's a dev run. A dev run is any work item where mika-dev launches claude-pilot, regardless of originating source.

## Solution

Changed `source = 'self_dev'` to `source IN ('self_dev', 'github_issue')` in all 5 SQL strings across both functions. Also:

- Added `source: Option<String>` to `DevRunResponse` API struct and frontend `DevRun` type
- Added a source badge column to the dev runs table (blue "Issue" / purple "Self Dev")
- Updated `docs/runtime-structure.md` to reflect the expanded filter

## Prevention

This is an instance of the **overly-restrictive filter** anti-pattern, previously seen with the delegate `channel_type` taxonomy expansion (see `docs/solutions/architecture-patterns/delegate-channel-type-taxonomy.md`).

**When adding filter-based dashboard views:**
- Consider all current and future values the filtered column can take
- The `source` column has no CHECK constraint — it's free-text, so new values can appear without migration
- If a hardcoded `IN (...)` list is used, document which values are included and why, so future values trigger a review

## Related

- `docs/solutions/architecture-patterns/delegate-channel-type-taxonomy.md` — same class of bug (filter too narrow for taxonomy)
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — duplicated SQL fragments causing drift
- `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — introduced the `source` column
