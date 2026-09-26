---
status: pending
priority: p2
issue_id: "684"
tags: [code-review, quality]
dependencies: []
---

# Duplicate agent_home binding in agents.rs create function

## Problem Statement

`agent_home` is computed twice in `create()`: once inside the `if interactive` block (line 63) and once unconditionally after it (line 87). The second binding shadows the first (which is scoped to the if-block). This is a minor DRY violation.

## Findings

- **Code Simplicity Reviewer**: Hoist the binding before the `if interactive` block to eliminate duplication.

**Affected files:**
- `crates/mika-cli/src/commands/agents.rs` (lines 63, 87)

## Proposed Solutions

Move `let agent_home = mika_common::agent::agent_dir(global_home, &name);` before the `if interactive` block and use it in both paths.

- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Single `agent_home` binding used in both wizard and seed paths
