---
status: complete
priority: p2
issue_id: "414"
tags: [code-review, security, performance, reflection]
dependencies: []
---

# Memory Events Digest Has No Size Cap

## Problem Statement

The conversation digest is properly capped at 50,000 characters in `agent.rs:1093`, but the memory events digest (`agent.rs:1103-1118`) has no equivalent cap. The `get_memory_events_since` query also has no LIMIT clause. Each memory event includes `before_value` and `after_value` fields that can be up to 10,000 characters each (MAX_INPUT_LEN).

## Findings

- **Security sentinel**: "In a pathologically active day with many memory tool calls, the after_value field can be up to 10,000 characters per event... could produce a memory events digest exceeding acceptable system prompt sizes"
- **Performance oracle**: Corroborated — unbounded system prompt growth is a cost and reliability risk

## Proposed Solutions

### Option A: Add 50K char truncation to memory events digest (Recommended)
Apply the same pattern used for conversations:
```rust
if buf.len() + line.len() > 50_000 {
    buf.push_str("... (truncated)\n");
    break;
}
```
- **Effort**: Small (5 lines)
- **Risk**: Low

## Technical Details

- **Affected file**: `crates/mika-agent/src/agent.rs` (lines 1103-1118)

## Acceptance Criteria

- [ ] Memory events digest has a character cap (e.g., 50K or 10K)
- [ ] Truncation marker shown when cap is hit
