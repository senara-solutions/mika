---
status: complete
priority: p1
issue_id: 523
tags: [code-review, logic-error, teams]
dependencies: []
---

# Label Mismatch Prevents Child Results Injection on Team Resume

## Problem Statement

When `dispatch_invoke_orchestrator` builds `child_results`, it uses `c.label` from child tasks (which is `"team-agent-{name}"`). When `execute_from_phase` tries to match these results against task assignments, it compares `result["agent"]` against `task.agent` — but task assignments store just `"{name}"` (e.g., "writer"), not `"team-agent-writer"`.

This means no task assignments will match, and no agent outputs will be injected on resume. The review and deliver phases will proceed with empty agent results.

**Severity:** P1 — Functional bug that silently breaks team suspend/resume.

## Findings

- `crates/mika-agent/src/teams/engine.rs` line ~727 — child task created with label `format!("team-agent-{}", agent_name)`
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `dispatch_invoke_orchestrator` builds child_results with `"agent": c.label`
- `crates/mika-agent/src/teams/engine.rs` `execute_from_phase` — matches `result["agent"]` against `task.agent` (which is just the agent name)

## Proposed Solutions

1. **Strip "team-agent-" prefix when building child_results in dispatcher**
   - In `dispatch_invoke_orchestrator`, use `c.label.strip_prefix("team-agent-").unwrap_or(&c.label)`
   - Pros: Simple, minimal change
   - Cons: Couples dispatcher to label format convention
   - Effort: Small
   - Risk: Low

2. **Store agent name separately in child task action_config**
   - When creating child tasks, store `agent_name` in `action_config` JSON
   - In dispatcher, read from `action_config["agent_name"]` instead of `c.label`
   - Pros: Clean separation, no string parsing
   - Cons: More plumbing
   - Effort: Small
   - Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/teams/engine.rs`, `crates/mika-agent/src/task_engine/dispatcher.rs`
- **Components:** Team engine, task dispatcher

## Acceptance Criteria

- [ ] Agent results are correctly matched to task assignments on resume
- [ ] Test verifies child results injection works end-to-end
