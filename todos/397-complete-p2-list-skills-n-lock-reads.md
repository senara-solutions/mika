---
status: complete
priority: p2
issue_id: "397"
tags: [code-review, performance, marketplace, pr-56]
dependencies: []
---

# list_skills tool reads lock file N times (once per skill)

## Problem Statement

`is_marketplace_skill()` reads and parses the entire `marketplace.lock` file from disk on every call. In `list_skills.rs`, this is called once per non-bundled skill, resulting in N file reads + N TOML parses per invocation. This tool runs in the agent loop (called by the LLM).

## Findings

- **Source**: performance-oracle, architecture-strategist, code-simplicity-reviewer (all flagged independently)
- **File**: `crates/mika-agent/src/tools/list_skills.rs:54-60`
- **Evidence**: `is_marketplace_skill` at `marketplace.rs:82-85` calls `read_lock()` which reads from disk every time

## Proposed Solutions

### Option A: Read lock once before loop (Recommended)

```rust
let lock = marketplace::read_lock(ctx.home_dir);
// In loop:
let origin = if is_bundled_skill(name) {
    " [built-in]"
} else if lock.skills.contains_key(name) {
    " [marketplace]"
} else {
    " [custom]"
};
```

- Pros: 3-line refactor, O(1) disk reads
- Cons: None
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Lock file read exactly once in `list_skills` execute
- [ ] Origin detection still works correctly
- [ ] Tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-03 | Created from PR #56 code review | Flagged by 3 agents independently |

## Resources

- `crates/mika-agent/src/tools/list_skills.rs:54-60`
- `crates/mika-agent/src/skills/marketplace.rs:82-85`
