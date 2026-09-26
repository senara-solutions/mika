---
status: complete
priority: p2
issue_id: "330"
tags: [code-review, architecture, quality]
dependencies: []
---

# Duplicate Routing Logic Between Text and Photo Handlers

## Problem Statement

`handle_text_message` and `handle_photo_message` share ~80 lines of identical logic: customer lookup, suspended check, dedup claim/reset, container URL computation, and error handling. This creates a risk of the two functions silently diverging.

## Findings

- Flagged by: simplicity-reviewer, architecture-strategist
- Location: `crates/mika-gateway/src/routes.rs` lines 204-296 vs 305-461

## Proposed Solutions

### Option A: Extract helpers (resolve_customer, claim_dedup, reset_dedup)
- **Pros:** ~60-80 LOC reduction, prevents divergence
- **Cons:** Slightly more indirection
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] Common logic extracted into shared helpers
- [ ] Both handlers use the shared helpers
- [ ] All existing tests still pass
