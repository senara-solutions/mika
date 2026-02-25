---
status: pending
priority: p1
issue_id: 248
tags: [code-review, security]
dependencies: []
---

# Write Workspace Path Containment Bypass

## Problem Statement

`write_workspace.rs` uses string prefix comparison (`to_string_lossy().starts_with()`) for path containment instead of `canonicalize()` + `Path::starts_with()`. This is weaker than `read_workspace.rs` which uses canonicalize. A string prefix check can be fooled by paths like `/tmp/workspace_evil` matching prefix `/tmp/workspace`.

## Findings

- **File:** `crates/mika-agent/src/tools/write_workspace.rs` lines 73-81
- **Severity:** P1 (Critical)
- **PR:** [#13](https://github.com/senara-solutions/mika/pull/13)

The write workspace tool constructs a path and checks containment using `to_string_lossy().starts_with()` on the string representation. This is a fundamentally weaker check than the canonicalize-based approach used in `read_workspace.rs`. String prefix matching treats `/tmp/workspace_evil` as a valid subdirectory of `/tmp/workspace` because the string starts with the same prefix. This allows writes outside the intended workspace directory.

## Proposed Solutions

After creating directories, canonicalize the full path and `workspace_dir`, then use `Path::starts_with()` (not string prefix). Match the pattern already established in `read_workspace.rs`:

```rust
let canonical_path = tokio::fs::canonicalize(&full_path).await?;
let canonical_workspace = tokio::fs::canonicalize(&workspace_dir).await?;
if !canonical_path.starts_with(&canonical_workspace) {
    anyhow::bail!("Path escapes workspace directory");
}
```

## Technical Details

- `Path::starts_with()` performs component-by-component comparison, not string prefix matching
- `canonicalize()` resolves symlinks and `..` components, giving the true filesystem path
- The write tool must canonicalize after `create_dir_all` since the target file may not exist yet, but parent directories will
- `read_workspace.rs` already implements this correctly and serves as the reference pattern

## Acceptance Criteria

- [ ] `write_workspace.rs` uses `canonicalize()` + `Path::starts_with()` for containment check
- [ ] String-based `starts_with()` check is removed
- [ ] Test with a path that shares a prefix with the workspace dir (e.g., `workspace_evil`) and verify it is rejected
- [ ] Test with a legitimate nested path inside the workspace and verify it is accepted
- [ ] Pattern matches the approach used in `read_workspace.rs`

## Work Log

- 2026-02-25: Finding identified during code review of PR #13

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Rust `Path::starts_with` docs: https://doc.rust-lang.org/std/path/struct.Path.html#method.starts_with
- Rust `fs::canonicalize` docs: https://doc.rust-lang.org/std/fs/fn.canonicalize.html
