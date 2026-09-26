---
status: complete
priority: p1
issue_id: 225
tags: [code-review, security, slash-commands]
dependencies: []
---

# Symlink Attack in /export Handler

## Problem Statement

The `/export` command in `handlers.rs` writes conversation exports to `~/.mika/exports/` without checking for symlink attacks. An attacker who can create a symlink at the export path could cause Mika to overwrite arbitrary files on the filesystem.

**Why it matters:** File write to attacker-controlled path via symlink is a classic local privilege escalation / data destruction vector.

## Findings

**Source:** Security Sentinel review agent

**Location:** `crates/mika-cli/src/tui/commands/handlers.rs:259-305` (`handle_export`)

The function constructs a path under `exports_dir` and writes directly via `tokio::fs::write()` without:
1. Checking if the target path is a symlink
2. Validating the path doesn't escape the exports directory
3. Using `O_NOFOLLOW` or equivalent safe file creation

```rust
let filepath = exports_dir.join(&filename);
// No symlink check here
match tokio::fs::write(&filepath, content).await {
```

## Proposed Solutions

### Solution A: Check and reject symlinks before write (Recommended)
- Before writing, check `tokio::fs::symlink_metadata()` and reject if path exists and is a symlink
- Also canonicalize the exports_dir and verify the final path is within it
- **Pros:** Simple, direct fix
- **Cons:** TOCTOU race (small window between check and write)
- **Effort:** Small
- **Risk:** Low

### Solution B: Use OpenOptions with create_new
- Use `tokio::fs::OpenOptions::new().create_new(true).write(true)` to fail if file exists
- Combined with timestamped filenames, collision is near-impossible
- **Pros:** Atomic, no TOCTOU race
- **Cons:** Fails if file already exists (unlikely with timestamp)
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Solution B — `create_new(true)` is the safest approach, atomic and no race condition.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/commands/handlers.rs`
- **Components:** Export handler

## Acceptance Criteria

- [ ] Export handler uses safe file creation (no symlink following)
- [ ] Path traversal prevented (filename can't escape exports dir)
- [ ] Test added for symlink rejection or create_new behavior

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Security sentinel flagged symlink attack vector |

## Resources

- PR branch: `feat/slash-commands`
- OWASP Path Traversal: https://owasp.org/www-community/attacks/Path_Traversal
