---
status: pending
priority: p2
issue_id: 270
tags: [code-review, quality, correctness]
dependencies: []
---

# Replace hand-rolled TOML parser with toml crate

## Problem Statement

`parse_log_level` in `main.rs` uses a hand-rolled line scanner that has edge cases: `starts_with("log_level")` matches prefix-similar keys like `log_level_override`, doesn't handle inline TOML comments, and doesn't handle unquoted values. The `toml` crate is already a dependency of `mika-cli`.

## Findings

- **File**: `crates/mika-cli/src/main.rs:85-98`
- **Impact**: Medium — false-match risk on similar key names, comment handling bug
- **Found by**: security-sentinel, code-simplicity-reviewer, pattern-recognition-specialist

## Proposed Solutions

Replace the 12-line scanner with a 3-line `toml` crate call:

```rust
fn parse_log_level(content: &str) -> Option<String> {
    let table: toml::Table = content.parse().ok()?;
    table.get("log_level")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
```

## Acceptance Criteria

- [ ] `parse_log_level` uses `toml::Table` instead of line scanning
- [ ] All existing tests pass unchanged
- [ ] Handles inline comments correctly
- [ ] Does not match prefix-similar keys
