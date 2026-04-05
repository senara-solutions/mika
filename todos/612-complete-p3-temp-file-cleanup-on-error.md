---
status: complete
priority: p3
issue_id: 612
tags: [code-review, security, robustness]
dependencies: []
---

# Temp file left behind on permission-setting failure

## Problem Statement

In `set_env_var` (dotenv.rs:68-72) and `set_config_toml_value` (setup.rs:378-382), if `std::fs::set_permissions` on the temp file fails, the function returns an error but leaves `.env.tmp` or `config.toml.tmp` on disk with potentially default permissions containing secret values.

## Findings

- **Source:** security-sentinel agent
- **Location:** `crates/mika-common/src/dotenv.rs:68-72`, `crates/mika-cli/src/commands/setup.rs:378-382`
- **Evidence:** No cleanup code after `set_permissions` error; `?` propagates immediately

## Proposed Solutions

### Option A: Cleanup on error (Recommended)
```rust
if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)) {
    let _ = std::fs::remove_file(&tmp_path);
    return Err(e.into());
}
```
- Effort: Small (4 lines per call site)
- Risk: Low

### Option B: Use a Drop guard
Wrap the temp file in a struct that removes it on drop unless `.persist()` is called (similar to `tempfile::NamedTempFile`).
- Effort: Medium
- Risk: Low — more robust but heavier

## Acceptance Criteria

- [ ] Temp files are removed if permission-setting fails
- [ ] Both `set_env_var` and `set_config_toml_value` cleaned up
