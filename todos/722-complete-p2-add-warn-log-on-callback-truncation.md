---
status: pending
priority: p2
issue_id: 722
tags: [code-review, quality, observability]
dependencies: []
---

# Add warn! log when callback result truncation activates

## Problem Statement

When `format_callback_framing()` truncates an oversized callback result, it does so silently with no log output. The project convention (per `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md` and `docs/solutions/integration-issues/skill-prompt-snippet-size-limit-configurable.md`) is to log when safety-net truncation activates so operators can diagnose why an agent received partial callback data.

## Findings

- `format_callback_framing()` in `crates/mika-agent/src/agent.rs` truncates results > 10KB but emits no log
- Other truncation paths in the codebase (tool metadata, skill prompts, compaction) all log when they activate
- Without a log, operators cannot easily determine whether a callback result was truncated vs. genuinely short

## Proposed Solutions

### Solution 1: Add warn! log with original and truncated sizes (Recommended)
```rust
warn!(
    original_bytes = result.len(),
    truncated_to = CALLBACK_RESULT_MAX_BYTES,
    "callback result truncated before prompt injection"
);
```
- **Pros**: Simple, follows project convention, gives operators the data they need
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Recommended Action

Solution 1.

## Technical Details

- **Affected files**: `crates/mika-agent/src/agent.rs` (format_callback_framing function)
- **Components**: Agent loop, callback delivery

## Acceptance Criteria

- [ ] `warn!` log emitted when callback result exceeds `CALLBACK_RESULT_MAX_BYTES`
- [ ] Log includes original byte count and truncation limit

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-24 | Created from code review of #259 | Project convention: always log safety-net truncations |

## Resources

- PR: #259 (callback result truncation)
- Learnings: `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md`
- Learnings: `docs/solutions/integration-issues/skill-prompt-snippet-size-limit-configurable.md`
