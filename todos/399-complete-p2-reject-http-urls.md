---
status: complete
priority: p2
issue_id: "399"
tags: [code-review, security, marketplace, pr-56]
dependencies: []
---

# Git clone allows http:// URLs (no TLS enforcement)

## Problem Statement

`resolve_url()` passes `http://` URLs through unchanged. Git clone over plain HTTP is vulnerable to man-in-the-middle attacks where an attacker on the network can inject arbitrary skill code during installation.

## Findings

- **Source**: security-sentinel
- **File**: `crates/mika-agent/src/skills/git.rs:85-86`

## Proposed Solutions

### Option A: Reject http:// with clear error (Recommended)

```rust
if source.starts_with("http://") {
    bail!("Insecure URL: '{source}'. Use https:// instead. Plain HTTP is vulnerable to man-in-the-middle attacks.");
}
```

- Effort: Small (3 lines)
- Risk: Low (could break users with http-only mirrors, but this is a security improvement)

### Option B: Warn but allow

Print a warning and proceed. Less secure but more permissive.

## Acceptance Criteria

- [ ] `http://` URLs rejected or warned
- [ ] Test for http:// URL handling updated
- [ ] `https://` URLs still work

## Resources

- `crates/mika-agent/src/skills/git.rs:85-86`
