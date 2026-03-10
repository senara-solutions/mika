---
status: complete
priority: p3
issue_id: 608
tags: [code-review, robustness, consistency]
dependencies: []
---

# config.toml writes lack atomic write pattern

## Problem Statement

`set_config_toml_value()` in `setup.rs` uses direct `std::fs::write()` for `config.toml`, while the `.env` writer (`mika_common::dotenv::set_env_var`) uses atomic write-to-temp-then-rename. A crash during write could corrupt `config.toml`. Additionally, `bootstrap_fresh_install()` in `home.rs` creates `config.toml` with 0600 permissions, but `set_config_toml_value` resets permissions to the default umask (typically 0644).

## Findings

- **Source:** architecture-strategist + pattern-recognition-specialist + security-sentinel agents
- **Location:** `crates/mika-cli/src/commands/setup.rs` — `set_config_toml_value()` line 180
- **Evidence:** `dotenv::set_env_var` uses temp+rename pattern (dotenv.rs:64-80); `set_config_toml_value` uses plain `fs::write`

## Proposed Solutions

### Option A: Match the dotenv atomic write pattern (Recommended)
```rust
let tmp_path = config_path.with_file_name("config.toml.tmp");
std::fs::write(&tmp_path, &content)?;
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
}
std::fs::rename(&tmp_path, &config_path)?;
```
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] `config.toml` writes use atomic temp-file-then-rename
- [x] File permissions preserved at 0600 on Unix
