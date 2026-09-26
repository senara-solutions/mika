---
status: complete
priority: p2
issue_id: 526
tags: [code-review, architecture, task-engine]
dependencies: []
---

# Fragile String-Matching for "Agent Busy" Error Detection

## Problem Statement

Three call sites detect the "agent busy" condition by checking `e.to_string().contains("agent busy")`. If the error message wording changes in `dispatch_resume_agent`, all callers silently break and tasks get permanently marked as failed instead of retried.

**Severity:** P2 — Fragile error contract that will break silently on refactoring.

## Findings

- `crates/mika-agent/src/task_engine/dispatcher.rs:205` — source: `anyhow!("agent busy, defer resume_agent task {}", task.id)`
- `crates/mika-agent/src/task_engine/engine.rs:432` — consumer: `err_msg.contains("agent busy")`
- `crates/mika-agent/src/server/handlers.rs:421` — consumer: `err_msg.contains("agent busy")`

## Proposed Solutions

1. **Typed error variant with thiserror**
   - Define `enum DispatchError { AgentBusy(String), Other(anyhow::Error) }` in dispatcher
   - Callers downcast to check variant
   - Pros: Compile-time safe, refactoring-proof
   - Cons: Adds a new error type
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] "Agent busy" detection uses typed error, not string matching
- [ ] All 3 call sites updated
