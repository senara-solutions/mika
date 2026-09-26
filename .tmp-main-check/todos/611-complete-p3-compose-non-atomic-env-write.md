---
status: complete
priority: p3
issue_id: 611
tags: [code-review, security, consistency]
dependencies: []
---

# Compose mode writes .env without atomic write pattern

## Problem Statement

`run_compose_generation()` in setup.rs writes the `.env` file using `std::fs::write` directly (line 305), then sets 0600 permissions afterwards (line 311). There is a brief window where the file containing API keys, Telegram tokens, and auth tokens is world-readable with default umask permissions. This deviates from the atomic write pattern (temp file, set perms, rename) used by `set_env_var` and `set_config_toml_value` in the same PR.

## Findings

- **Source:** security-sentinel, pattern-recognition agents
- **Location:** `crates/mika-cli/src/commands/setup.rs:305-311`
- **Evidence:** `std::fs::write` creates file with umask perms; `set_permissions(0o600)` called after

## Proposed Solutions

### Option A: Use OpenOptions with mode (Recommended)
```rust
#[cfg(unix)]
{
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true)
        .mode(0o600)
        .open(&env_path)?
        .write_all(content.as_bytes())?;
}
#[cfg(not(unix))]
std::fs::write(&env_path, &content)?;
```
- Effort: Small
- Risk: Low

### Option B: Use same temp-file-rename pattern as set_env_var
- Effort: Small
- Risk: Low — consistent with existing pattern

## Acceptance Criteria

- [ ] Compose .env file is never world-readable, even briefly
- [ ] Pattern matches `set_env_var` or uses OpenOptions with mode
