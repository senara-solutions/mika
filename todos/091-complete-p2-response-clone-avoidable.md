---
status: complete
priority: p2
issue_id: "091"
tags: [code-review, performance]
dependencies: []
---

# Remove unnecessary response.clone() in run_agent

## Problem Statement
`run_agent` in agent.rs uses `Ok(Ok(ref response))` which borrows the String, then requires `.clone()` to return it. Since `maybe_compact` does not use the response, the `ref` binding is unnecessary.

## Findings
- File: `crates/mika-agent/src/agent.rs:47-53`
- `ref response` borrows the String, forcing a `.clone()` to return ownership
- `maybe_compact(params.db, params.claude)` does not reference the response
- Removing `ref` allows moving the String directly without allocation
- Flagged by: Pattern Recognition (P2)

## Proposed Solutions

### Option 1: Remove ref binding (Recommended)
```rust
Ok(Ok(response)) => {
    if let Err(e) = compaction::maybe_compact(params.db, params.claude).await {
        warn!(error = %e, "post-turn compaction failed");
    }
    Ok(response)
}
```
**Pros:** Zero-cost, eliminates one String allocation per agent turn
**Effort:** Trivial
**Risk:** None

## Technical Details
**Affected files:** `crates/mika-agent/src/agent.rs`

## Acceptance Criteria
- [ ] `response.clone()` removed in `run_agent`
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified unnecessary String clone in agent response path
