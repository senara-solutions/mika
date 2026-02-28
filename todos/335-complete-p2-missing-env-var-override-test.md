---
status: complete
priority: p2
issue_id: "335"
tags: [code-review, testing]
dependencies: []
---

# Missing Env Var Override Test for disable_bundled_skills

## Problem Statement

The `disable_bundled_skills` setting is primarily intended to be set via the `MIKA_DISABLE_BUNDLED_SKILLS` environment variable. However, there is no test verifying that setting this env var actually flips the boolean to `true`. The only test coverage is `assert!(!settings.disable_bundled_skills)` in `test_defaults`, which only verifies the default value.

Other settings like `claude_model` and `db_path` have dedicated env var override tests.

## Findings

- Flagged by: pattern-recognition-specialist
- Location: `crates/mika-common/src/config.rs` (tests module)
- Existing pattern: `test_env_overrides_home_config` tests env var override for `claude_model`
- Missing: equivalent test for `MIKA_DISABLE_BUNDLED_SKILLS=true`

## Proposed Solutions

### Option A: Add env var override test
```rust
#[test]
#[serial]
fn test_disable_bundled_skills_from_env() {
    clean_env();
    unsafe { std::env::set_var("MIKA_DISABLE_BUNDLED_SKILLS", "true") };
    let tmp = tempfile::tempdir().unwrap();
    let settings = Settings::load(tmp.path()).unwrap();
    assert!(settings.disable_bundled_skills);
    unsafe { std::env::remove_var("MIKA_DISABLE_BUNDLED_SKILLS") };
}
```
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Test exists that sets `MIKA_DISABLE_BUNDLED_SKILLS=true` and verifies `settings.disable_bundled_skills == true`
