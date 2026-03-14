---
status: pending
priority: p2
issue_id: 668
tags: [code-review, documentation, architecture]
dependencies: []
---

# Document Team Engine message_sender: None Intent

## Problem Statement

Both team engine call sites set `message_sender: None` without explaining why. Future contributors may "fix" this by wiring a sender through, which would break the team architecture (agents should communicate through the orchestrator pipeline, not directly to users).

## Findings

- `crates/mika-agent/src/teams/engine.rs:891` — `message_sender: None`
- `crates/mika-agent/src/teams/engine.rs:1216` — `message_sender: None`

Identified by: architecture-strategist, agent-native-reviewer, code-simplicity-reviewer

## Proposed Solutions

Add comments at both sites:

```rust
// Team agents communicate through the orchestrator pipeline (workspace entries,
// deliverables, critic feedback), not directly to users via Telegram.
message_sender: None,
```

- **Effort**: Small
- **Risk**: None
