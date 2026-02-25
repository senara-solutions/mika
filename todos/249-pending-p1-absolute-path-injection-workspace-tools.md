---
status: pending
priority: p1
issue_id: 249
tags: [code-review, security]
dependencies: []
---

# Absolute Path Injection in Workspace Tools

## Problem Statement

Workspace tools don't check if the user-supplied path is absolute. Rust's `Path::join()` replaces the entire path when given an absolute path. For example, `workspace_dir.join("/etc/passwd")` becomes `/etc/passwd`. The existing `..` check doesn't catch this. While `read_workspace.rs`'s canonicalize guard catches it incidentally, `write_workspace.rs` relies on the weak string-prefix check and is fully exploitable.

## Findings

- **Files:**
  - `crates/mika-agent/src/tools/read_workspace.rs` line 50
  - `crates/mika-agent/src/tools/write_workspace.rs` line 63
  - `crates/mika-agent/src/tools/list_workspace.rs`
- **Severity:** P1 (Critical)
- **PR:** [#13](https://github.com/senara-solutions/mika/pull/13)

Rust's `Path::join()` has documented behavior where joining an absolute path discards the base entirely. An LLM-generated or prompt-injected path of `/etc/passwd` would bypass the relative path assumption made by all three workspace tools. Additionally, the current `..` check uses `path.contains("..")` which is a string substring match. This incorrectly blocks valid filenames like `file..v2.md` or `notes...draft.txt` while also being bypassable in edge cases.

## Proposed Solutions

1. Add an explicit absolute path check at the top of each workspace tool's `execute()`:

```rust
if std::path::Path::new(path).is_absolute() {
    anyhow::bail!("Absolute paths are not allowed; use a relative path within the workspace");
}
```

2. Refactor the `..` check to use `Path::components()` to detect `Component::ParentDir` specifically:

```rust
use std::path::Component;

if std::path::Path::new(path).components().any(|c| matches!(c, Component::ParentDir)) {
    anyhow::bail!("Path traversal via '..' is not allowed");
}
```

## Technical Details

- `Path::join()` behavior is documented: "If path is absolute, it replaces the current path"
- `path.contains("..")` is a naive string check that matches substrings in filenames
- `Path::components()` parses the path into typed components, distinguishing `ParentDir` (`..`) from normal path segments
- The absolute path check should come before `join()` to prevent the base path from being discarded
- All three workspace tools (`read_workspace`, `write_workspace`, `list_workspace`) need this fix

## Acceptance Criteria

- [ ] All 3 workspace tools reject absolute paths with a clear error message
- [ ] Test with `/etc/passwd` as input path and verify rejection
- [ ] `..` check refactored to use `Path::components()` and `Component::ParentDir`
- [ ] `file..v2.md` is accepted as a valid filename
- [ ] `../escape` is still correctly rejected
- [ ] Tests cover both absolute path and parent directory traversal cases

## Work Log

- 2026-02-25: Finding identified during code review of PR #13

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Rust `Path::join` docs: https://doc.rust-lang.org/std/path/struct.Path.html#method.join
- Rust `Path::components` docs: https://doc.rust-lang.org/std/path/struct.Path.html#method.components
