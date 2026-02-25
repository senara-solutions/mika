---
status: pending
priority: p1
issue_id: 250
tags: [code-review, security]
dependencies: []
---

# Symlink Attacks in Workspace Tools

## Problem Statement

Workspace tools follow symlinks, allowing escape from the workspace directory. `ReadWorkspaceTool` has a TOCTOU (time-of-check-time-of-use) gap between the canonicalize check and the actual read. `ListWorkspaceTool`'s `collect_files` uses `path.is_dir()`/`path.is_file()` which follow symlinks, leaking file listings outside the workspace. `WriteWorkspaceTool` creates parent directories that could race with symlink creation.

## Findings

- **Files:**
  - `crates/mika-agent/src/tools/read_workspace.rs` lines 53-88
  - `crates/mika-agent/src/tools/list_workspace.rs` lines 56-77
  - `crates/mika-agent/src/tools/write_workspace.rs`
- **Severity:** P1 (Critical)
- **PR:** [#13](https://github.com/senara-solutions/mika/pull/13)

All three workspace tools follow symlinks without detection. Specific attack scenarios:

1. **read_workspace:** An attacker places a symlink in the workspace pointing to `/etc/shadow`. The canonicalize check resolves the symlink and may pass if the target exists, then the file is read. Even if canonicalize catches it, there is a TOCTOU window where the symlink could be swapped between check and read.

2. **list_workspace:** The `collect_files` function uses `path.is_dir()` and `path.is_file()` which transparently follow symlinks. A symlink to `/home` would cause the tool to recursively list files outside the workspace.

3. **write_workspace:** The `create_dir_all` step could race with symlink creation, and the subsequent write follows the symlink to an arbitrary location.

## Proposed Solutions

Add symlink detection in all three workspace tools:

**list_workspace.rs** — Use `entry.file_type()` from `ReadDir` and skip symlinks:
```rust
let ft = entry.file_type().await?;
if ft.is_symlink() {
    continue;
}
```

**read_workspace.rs** — Check symlink metadata before reading:
```rust
let metadata = tokio::fs::symlink_metadata(&canonical_path).await?;
if metadata.file_type().is_symlink() {
    anyhow::bail!("Symlinks are not allowed in workspace");
}
```

**write_workspace.rs** — Check that no component of the resolved path is a symlink:
```rust
let metadata = tokio::fs::symlink_metadata(&full_path).await;
if let Ok(m) = metadata {
    if m.file_type().is_symlink() {
        anyhow::bail!("Cannot write to symlink targets");
    }
}
```

## Technical Details

- `symlink_metadata()` (or `lstat` on Unix) returns metadata about the symlink itself, not its target
- `metadata()` (or `stat`) follows symlinks and returns metadata about the target
- `is_dir()` and `is_file()` follow symlinks; `symlink_metadata().file_type().is_symlink()` detects them
- TOCTOU gaps are inherent in check-then-act patterns on filesystems; symlink detection reduces but does not fully eliminate the race window
- For defense in depth, combine symlink detection with the canonicalize + `Path::starts_with()` check

## Acceptance Criteria

- [ ] All three workspace tools detect and reject/skip symlinks
- [ ] `list_workspace` uses `entry.file_type()` and skips entries where `is_symlink()` is true
- [ ] `read_workspace` checks `symlink_metadata` before reading file content
- [ ] `write_workspace` checks `symlink_metadata` before writing
- [ ] Test with a symlink pointing outside the workspace directory and verify it is rejected/skipped
- [ ] Regular files and directories continue to work normally

## Work Log

- 2026-02-25: Finding identified during code review of PR #13

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Rust `symlink_metadata` docs: https://doc.rust-lang.org/std/fs/fn.symlink_metadata.html
- TOCTOU race conditions: https://en.wikipedia.org/wiki/Time-of-check_to_time-of-use
