---
status: complete
priority: p2
issue_id: 300
tags: [code-review, architecture, simplification]
dependencies: []
---

# Replace BUNDLED_SKILL_NAMES constant with is_bundled_skill() function

## Problem Statement

`BUNDLED_SKILL_NAMES` is a manually-synchronized duplicate of data already in `BUNDLED_SKILLS`. A developer adding a new bundled skill must update both. The sync test catches drift at test time, but the root cause (data duplication) can be eliminated.

## Findings

- **Architecture Strategist:** The dependency direction is tools/list_skills.rs -> bundled_skills.rs. `BundledSkill` is intentionally private. Exposing just a predicate function is the minimal public API.
- **Code Simplicity Reviewer:** Replacing the constant with a function eliminates the constant, the sync test, and the doc comment. Net -4 lines, strictly safer.
- **Performance Oracle:** 5-element linear scan is ~100ns. No difference between const slice and function-based iteration.

## Proposed Solutions

### Solution A: Replace with `is_bundled_skill()` predicate function

In `bundled_skills.rs`, replace:
```rust
pub const BUNDLED_SKILL_NAMES: &[&str] = &["tmux", "shell-exec", "web-search", "file-reader", "calendar"];
```

With:
```rust
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS.iter().any(|s| s.name == name)
}
```

In `list_skills.rs`, change:
```rust
use crate::bundled_skills::BUNDLED_SKILL_NAMES;
let origin = if BUNDLED_SKILL_NAMES.contains(&entry.manifest.skill.name.as_str()) {
```
To:
```rust
use crate::bundled_skills::is_bundled_skill;
let origin = if is_bundled_skill(&entry.manifest.skill.name) {
```

Remove sync test `test_bundled_skill_names_matches_bundled_skills`.

- Effort: Small
- Risk: Low

## Technical Details

- Files: `crates/mika-agent/src/bundled_skills.rs`, `crates/mika-agent/src/tools/list_skills.rs`
