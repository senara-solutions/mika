---
status: complete
priority: p2
issue_id: "336"
tags: [code-review, testing]
dependencies: []
---

# Missing Unit Test for seed_bundled_skills_if_needed Disabled Path

## Problem Statement

The `seed_bundled_skills_if_needed` function gained a `disabled: bool` parameter, but there is no test verifying that passing `true` actually skips seeding. The feature's core purpose (disabling seeding) has zero test coverage.

## Findings

- Flagged by: pattern-recognition-specialist
- Location: `crates/mika-agent/src/startup.rs:30-39`
- The existing bundled_skills tests in `crates/mika-agent/src/bundled_skills.rs` call `seed_bundled_skills` directly, not `seed_bundled_skills_if_needed`

## Proposed Solutions

### Option A: Add unit test for disabled path
```rust
#[test]
fn test_seed_bundled_skills_skipped_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    seed_bundled_skills_if_needed(tmp.path(), true);
    // Verify no skill directories were created
    assert_eq!(std::fs::read_dir(&skills_dir).unwrap().count(), 0);
}
```
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Test verifies `seed_bundled_skills_if_needed(path, true)` does not create any skill directories
- [ ] Test verifies `seed_bundled_skills_if_needed(path, false)` still seeds when skills dir exists
