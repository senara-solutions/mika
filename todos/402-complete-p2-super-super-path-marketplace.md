---
status: complete
priority: p2
issue_id: "402"
tags: [code-review, architecture, marketplace, pr-56]
dependencies: []
---

# Replace super::super:: path with crate:: in marketplace.rs

## Problem Statement

`marketplace.rs` uses `super::super::tools::create_skill::validate_skill_name` — a fragile double-super path. The codebase convention is `crate::` absolute paths (as used in `install.rs`).

Multiple agents also noted this indicates the validators may belong in a shared `skills/validation.rs` module rather than in `tools/create_skill.rs`.

## Findings

- **Source**: architecture-strategist (both instances)
- **File**: `crates/mika-agent/src/skills/marketplace.rs` (line using `super::super::`)

## Proposed Solutions

### Option A: Replace with crate:: path (Recommended, minimal)

Change to `crate::tools::create_skill::validate_skill_name` for consistency.

- Effort: Trivial
- Risk: None

### Option B: Extract validators to skills/validation.rs (Better long-term)

Move shared validators to a dedicated module, eliminating the reverse dependency from skills→tools.

- Effort: Medium
- Risk: Low (follow-up refactor)

## Acceptance Criteria

- [ ] No `super::super::` paths in marketplace.rs
- [ ] Consistent with rest of codebase

## Resources

- `crates/mika-agent/src/skills/marketplace.rs`
- `crates/mika-agent/src/skills/install.rs` (uses `crate::` path correctly)
