---
status: complete
priority: p3
issue_id: 295
tags: [code-review, security, hardening]
dependencies: []
---

# Handler script permissions 0o755 (world-readable/executable) instead of 0o700

## Problem Statement

Bundled skill handler scripts are set to `0o755` at `bundled_skills.rs:225`. On multi-user systems, any user can read and execute these scripts. While they contain no secrets and K8s containers are single-user, `0o700` would be consistent with the `bootstrap()` function which sets directories to `0o700` and files to `0o600`.

## Findings

- **Security Sentinel:** Minor hardening opportunity. Scripts contain generic handler logic (no secrets), but `0o700` matches the project's existing permission convention in `home.rs:198-223`.

## Proposed Solutions

### Solution A: Change to 0o700

```rust
std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o700))?;
```

- **Pros:** Consistent with existing permission conventions, slightly tighter security
- **Cons:** None
- **Effort:** Small (one-line change)
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/bundled_skills.rs`

## Acceptance Criteria

- [ ] Handler scripts set to 0o700 instead of 0o755
- [ ] `test_handlers_are_executable` still passes
