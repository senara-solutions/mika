---
status: complete
priority: p2
issue_id: "100"
tags: [code-review, agent-native, feature-gap]
dependencies: []
---

# Surface event context field in search_memory results

## Problem Statement
The `memory_events` table stores a `context` field with each event (e.g., "stored via store_fact tool", session context), but `search_memory` never includes this field in results returned to the agent. The agent stores context but can never retrieve it, creating a write-only data path.

## Findings
- File: `crates/mika-agent/src/tools/search_memory.rs`
- `memory_events` rows have: event_type, category, key, old_value, new_value, context, timestamp
- `search_memory` returns formatted results but omits `context`
- Context could help the agent understand *why* a fact was stored or changed
- Flagged by: Agent-Native Reviewer (Critical capability gap)

## Proposed Solutions

### Option 1: Include context in search results when non-empty (Recommended)
Append context to each result line when the context field is non-empty:
```rust
if !event.context.is_empty() {
    result.push_str(&format!(" (context: {})", event.context));
}
```
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/tools/search_memory.rs`

## Acceptance Criteria
- [ ] search_memory includes context field when non-empty
- [ ] Tests updated
- [ ] Agent can retrieve context it previously stored

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Agent-Native Reviewer identified write-only data path for event context
