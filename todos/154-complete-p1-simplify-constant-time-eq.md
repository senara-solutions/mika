---
status: complete
priority: p1
issue_id: "154"
tags: [code-review, security, simplicity]
---

# Simplify constant_time_eq — Remove Misleading Length-Padding

## Problem Statement
The `constant_time_eq` function in `routes.rs:361-373` attempts length-padding by comparing `a_bytes.ct_eq(a_bytes)` on mismatch, but this is misleading — the self-comparison doesn't prevent timing leaks on length. All 7 review agents flagged this. The `subtle` crate's `ct_eq` already handles unequal lengths correctly (returns 0 without leaking content).

## Findings
- **Security sentinel**: Length leak via early return; self-comparison doesn't match time profile of cross-comparison
- **Architecture strategist**: Inconsistent with agent's simpler `ct_eq` pattern at `server/auth.rs:20`
- **Code simplicity reviewer**: 13-line function can be replaced with 1 line; 4 tests are testing the `subtle` library
- **Learnings researcher**: Phase 2 docs recommend direct `ct_eq` usage

## Proposed Solutions

### Option A: One-liner using subtle directly (Recommended)
```rust
fn constant_time_eq(a: &str, b: &str) -> bool {
    subtle::ConstantTimeEq::ct_eq(a.as_bytes(), b.as_bytes()).into()
}
```
- Pros: Simple, correct, matches agent crate pattern
- Cons: None
- Effort: Small (15 min)
- Risk: None

### Option B: HMAC-based comparison
- Pros: Normalizes lengths via hashing
- Cons: Adds sha2/hmac dependency, over-engineered for this use case
- Effort: Medium
- Risk: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (lines 361-373, tests 411-428)
- **Components**: Webhook auth, Bearer token auth

## Acceptance Criteria
- [ ] `constant_time_eq` is ≤3 lines
- [ ] No misleading "length-padded" comments
- [ ] Reduce constant_time_eq tests to 1 smoke test
- [ ] All existing tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review (7-agent consensus)

## Resources
- PR: #6
- Agent crate pattern: `crates/mika-agent/src/server/auth.rs:20`
