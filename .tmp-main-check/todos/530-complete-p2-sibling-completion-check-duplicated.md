---
status: complete
priority: p2
issue_id: 530
tags: [code-review, architecture, duplication]
dependencies: []
---

# Sibling Completion Check Duplicated in 4 Places

## Problem Statement

The pattern `try_complete_parent_on_sibling_done` + dispatch parent is copy-pasted identically in 4 locations with the same log-and-dispatch block.

**Severity:** P2 — Duplication that will diverge over time.

## Findings

- `crates/mika-agent/src/server/handlers.rs:436` — inside resume_agent spawn
- `crates/mika-agent/src/server/handlers.rs:477` — non-resume_agent path
- `crates/mika-agent/src/task_engine/engine.rs:416` — dispatch worker success
- `crates/mika-agent/src/task_engine/engine.rs:231` — check_expired_siblings

## Proposed Solutions

1. **Extract helper method on TaskDispatcher**
   - `async fn check_and_dispatch_parent(&self, task_id: &str) -> Result<()>`
   - Pros: Single source of truth, consistent logging
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Single helper method replaces all 4 instances
- [ ] Behavior unchanged
