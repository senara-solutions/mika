---
status: complete
priority: p3
issue_id: "450"
tags: [code-review, performance]
dependencies: []
---

# Full Deliverables Fetched for Prompt Truncation

## Problem Statement

`load_team_runs` returns full deliverable text from DB, which is then truncated to 500 chars in Rust (`prompt.rs`). Could use `SUBSTR(deliverable, 1, 500)` in SQL.

## Proposed Fix

Add a `load_team_runs_for_prompt()` variant that uses `SUBSTR()` in SQL, or add an optional `truncate_deliverable` parameter.

## Acceptance Criteria

- [x] Deliverable truncation moved to SQL layer
