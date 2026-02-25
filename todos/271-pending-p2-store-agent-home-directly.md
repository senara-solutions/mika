---
status: pending
priority: p2
issue_id: 271
tags: [code-review, architecture, robustness]
dependencies: []
---

# Store agent_home directly instead of deriving via log_dir.parent()

## Problem Statement

In `main.rs`, the agent home directory is re-derived from `log_dir` using `.parent()` (stripping `/logs` suffix). This is fragile — if the log subdirectory naming ever changes, the derivation silently breaks.

## Findings

- **File**: `crates/mika-cli/src/main.rs:36-39`
- **Impact**: Low — works correctly today but fragile under future changes
- **Found by**: architecture-strategist, code-simplicity-reviewer

## Proposed Solution

Store `agent_home` as an explicit variable and derive `log_dir` from it:

```rust
let agent_home = global_home.as_ref().map(|h| home::resolve_agent_home(h, &agent_name));
let log_dir = agent_home.as_ref().map(|h| h.join("logs"));
```

## Acceptance Criteria

- [ ] `agent_home` is stored directly, no `.parent()` hack
- [ ] `log_dir` derived from `agent_home`
- [ ] Config reads use `agent_home` directly
