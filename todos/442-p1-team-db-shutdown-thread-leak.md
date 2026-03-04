---
status: complete
priority: p1
issue_id: "442"
tags: [code-review, resource-leak]
dependencies: []
---

# Missing team_db.shutdown() — Thread Leak

## Problem Statement

In `crates/mika-agent/src/teams/engine.rs:227-230`, only agent DBs are shut down after execution. The `self.team_db` `AsyncDatabase` is never shut down, leaking an OS thread per team run.

## Fix

Add `self.team_db.shutdown();` after the agent DB shutdown loop.

## Acceptance Criteria

- [ ] `team_db.shutdown()` called in `execute()`
