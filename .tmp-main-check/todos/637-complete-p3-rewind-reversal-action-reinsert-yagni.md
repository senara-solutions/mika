---
status: complete
priority: p3
issue_id: "637"
tags: [code-review, simplicity, rewind]
dependencies: []
---

# ReversalAction::Reinsert variant is never constructed (YAGNI)

## Problem Statement

`ReversalAction::Reinsert` exists in the enum but is never constructed anywhere in the codebase. It adds dead code and cognitive overhead.

## Findings

- **Source:** Simplicity review agent
- **Location:** `crates/mika-agent/src/rewind.rs` — `ReversalAction` enum
- The variant was likely added speculatively for future use
- It increases match arm count in every match on `ReversalAction`

## Proposed Solutions

### Option A: Remove the variant
Delete `Reinsert` from the enum and any match arms that handle it.
- **Effort:** Small
- **Risk:** None — it's never used

## Acceptance Criteria

- [ ] `ReversalAction::Reinsert` removed or justified with a comment explaining planned use
