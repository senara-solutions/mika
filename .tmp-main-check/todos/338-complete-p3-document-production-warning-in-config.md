---
status: complete
priority: p3
issue_id: "338"
tags: [code-review, security, documentation]
dependencies: []
---

# Document Production Warning for disable_bundled_skills

## Problem Statement

The `disable_bundled_skills` setting prevents security updates to handler scripts (shell-exec, file-reader, etc.) from propagating on restart. The current comment in `default.toml` says "useful for debugging handlers" but does not warn against production use. Operators could inadvertently set this in production and block future security patches.

## Findings

- Flagged by: security-sentinel
- Location: `config/default.toml:13-14`
- Current comment: `# Disable bundled skill re-sync on startup (useful for debugging handlers)`
- Handler scripts include shell-exec (arbitrary command execution) and file-reader (arbitrary file read)

## Proposed Solutions

### Option A: Enhance the comment with a warning
```toml
# Disable bundled skill re-sync on startup (useful for debugging handlers)
# WARNING: Do not enable in production — prevents security updates to handler scripts
# disable_bundled_skills = false
```
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Comment in `default.toml` warns against production use
- [ ] Optional: similar warning in `.env.example`
