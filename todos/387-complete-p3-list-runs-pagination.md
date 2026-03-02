---
status: complete
priority: p3
issue_id: 387
tags: [code-review, performance]
dependencies: []
---

# Add pagination to list_runs() in history module

## Problem Statement

`list_runs()` reads and parses ALL TOML history files before filtering/truncating. With 1000 runs, this means 1000 file reads for a `limit=5` query.

## Findings

- **Source:** Performance Oracle
- `get_team_history` calls `list_runs()` then `.take(limit)` after full load
- `get_team_status` with `run_id` also loads all runs to find one by ID

## Proposed Solutions

### Option 1: Add list_runs_limited() (Recommended)
Pass the limit into the file-reading loop so it stops after reading `limit` files. For run_id lookup, construct the filename directly.
- **Effort:** Small
- **Risk:** None
- Not urgent while run counts are low
