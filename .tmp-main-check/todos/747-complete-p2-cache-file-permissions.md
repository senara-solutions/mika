---
status: pending
priority: p2
issue_id: "747"
tags: [code-review, security]
dependencies: []
---

# Cache files written with default permissions

## Problem Statement

`write_cache()` in `models.rs` uses `std::fs::write()` without setting restrictive file permissions. Unlike `write_config_toml()` which correctly sets `0o600`, cache files inherit the process umask (typically `0o644` — world-readable). While cache files only contain model IDs and names, the `base_url` field could leak custom/internal API endpoints.

## Findings

- **File:** `crates/mika-common/src/llm/models.rs` — `write_cache()` function
- **Pattern violation:** Inconsistent with `write_config_toml()` in `crates/mika-cli/src/commands/config.rs` which sets `0o600`
- **Risk:** Low data sensitivity but breaks the project's consistent security posture for agent home directory files

## Proposed Solution

Add `#[cfg(unix)]` permission setting after the write:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
```

**Effort:** Small (5 lines)

## Acceptance Criteria

- [ ] Cache files at `{agent_home}/cache/models/*.json` are created with `0o600` permissions on Unix
- [ ] Existing tests still pass
