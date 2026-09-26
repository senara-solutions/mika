---
status: complete
priority: p3
issue_id: 539
tags: [code-review, duplication, teams]
dependencies: []
---

# execute_from_phase Duplicates Review-Deliver-Finalize Sequence

## Problem Statement

`execute_from_phase` duplicates the review, deliver, update_team_run, and shutdown sequence from `execute_inner`/`execute`. The DB update and shutdown blocks are copy-paste.

**Severity:** P3 — Duplication that will diverge.

## Findings

- `crates/mika-agent/src/teams/engine.rs:206-300` — `execute_from_phase`
- `crates/mika-agent/src/teams/engine.rs` — `execute_inner`/`execute` has same tail

## Proposed Solutions

1. **Extract shared finalization method**
   - `async fn finalize_run(&mut self) -> Result<TeamRun>` for review → deliver → update → shutdown
   - Effort: Medium
   - Risk: Low

## Acceptance Criteria

- [ ] Shared finalization path for both execute and execute_from_phase
