---
status: complete
priority: p2
issue_id: "443"
tags: [code-review, error-handling]
dependencies: []
---

# Silent `let _ =` on insert_team_message

## Problem Statement

Six sites in `crates/mika-agent/src/teams/engine.rs` discard errors from `insert_team_message` with `let _ =`. The nearby `insert_team_run` and `update_team_run` calls correctly use `warn!`.

## Fix

Replace `let _ = ...insert_team_message(...)` with `if let Err(e) = ...insert_team_message(...) { warn!(error = %e, "failed to persist team message"); }`.

## Acceptance Criteria

- [ ] All `insert_team_message` call sites log errors with `warn!`
