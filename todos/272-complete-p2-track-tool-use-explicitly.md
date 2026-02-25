---
status: complete
priority: p2
issue_id: 272
tags: [code-review, architecture, correctness]
dependencies: []
---

# Replace step > 0 guard with explicit tool_use_occurred flag

## Problem Statement

The `step > 0` guard on the empty-response `warn!` in `agent.rs` is an implicit invariant that relies on loop structure. The condition means "tool use occurred" only because the loop always progresses through ToolUse before reaching step > 0. A dedicated boolean would make the intent explicit.

## Findings

- **File**: `crates/mika-agent/src/agent.rs:193, 206`
- **Impact**: Low — correct today but fragile under loop changes
- **Found by**: architecture-strategist, security-sentinel, code-simplicity-reviewer

## Proposed Solution

```rust
let mut tool_use_occurred = false;
// In ToolUse arm:
tool_use_occurred = true;
// In EndTurn/StopSequence arms:
} else if tool_use_occurred {
    warn!(...);
}
```

Also add `stop_reason` to the StopSequence warn! for consistency.

## Acceptance Criteria

- [ ] Explicit `tool_use_occurred` boolean tracks tool execution
- [ ] Both EndTurn and StopSequence warn! include `stop_reason`
