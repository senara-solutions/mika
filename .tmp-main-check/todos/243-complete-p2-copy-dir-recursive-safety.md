---
status: complete
priority: p2
issue_id: "243"
tags: [code-review, security, safety]
dependencies: []
---

# `copy_dir_recursive` follows symlinks and has no depth limit

## Problem Statement

The `copy_dir_recursive` function in the clone command does not check for symlinks (could follow symlinks to copy files from outside the agents directory) and has no recursion depth limit (symlink cycles cause stack overflow).

## Findings

- **Source:** Security Sentinel, Performance Oracle
- **File:** `crates/mika-cli/src/commands/agents.rs:146-159`

## Proposed Solutions

Add symlink skip and depth limit:

```rust
fn copy_dir_recursive(src: &Path, dst: &Path, depth: u32) -> Result<()> {
    if depth > 10 { bail!("directory nesting too deep"); }
    // ...
    if ty.is_symlink() { continue; }
    // ...
}
```

## Acceptance Criteria

- [ ] Symlinks are skipped during copy
- [ ] Depth limit prevents stack overflow
- [ ] Caller updated to pass initial depth of 0

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Defense in depth for filesystem operations |
