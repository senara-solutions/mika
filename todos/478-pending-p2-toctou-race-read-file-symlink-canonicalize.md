---
status: pending
priority: p2
issue_id: "478"
tags: [code-review, security, tools]
dependencies: []
---

# TOCTOU Race Between symlink_metadata and canonicalize in read_file

## Problem Statement

`read_file` performs a two-step check: (1) `tokio::fs::symlink_metadata` to reject symlinks,
then (2) `Path::canonicalize` for containment verification. Between these two syscalls, the
filesystem is not locked. An attacker (or buggy code) that can race a regular-file → symlink
replacement in the fraction of a millisecond between the two calls could pass the first check
with a real file, have it replaced by a symlink before `canonicalize`, and reach the read with
a path that resolves outside the home directory. Additionally, `Path::canonicalize` at line 62
is the synchronous variant — it blocks the tokio thread.

## Findings

- **Source**: security-sentinel review
- **Location**: `crates/mika-agent/src/tools/read_file.rs:44–82`
- The same two-step pattern exists in `validate_and_resolve_path` for the parent directory
  (mod.rs:241–275)
- Race is difficult to exploit reliably (requires filesystem write access to agent home dir)
  but represents a gap between stated and actual security model

## Proposed Solutions

### Option A: O_NOFOLLOW-based open (Recommended)
Replace the check-then-open pattern with `std::fs::OpenOptions` with `custom_flags(libc::O_NOFOLLOW)`
or `openat2` with `RESOLVE_NO_SYMLINKS`. Use the open file descriptor for all subsequent operations.
- **Pros**: Atomically enforces no-symlinks, no race window
- **Cons**: Linux-specific API, adds `libc` dependency use
- **Effort**: Medium | **Risk**: Low

### Option B: Use tokio::fs::canonicalize (partial fix)
Replace `Path::canonicalize` with `tokio::fs::canonicalize` (async variant) to at least fix the
blocking call on the tokio thread. Does not eliminate the race window but is a safe improvement.
- **Pros**: Fixes blocking I/O issue, easy change
- **Cons**: Race window remains
- **Effort**: Small | **Risk**: None

### Option C: Accept current approach with documentation
Document the TOCTOU window as a known limitation (requires compromised home dir to exploit).
- **Pros**: No code change
- **Cons**: Security model documentation gap
- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] `symlink_metadata` + any subsequent read form an atomic unit (O_NOFOLLOW) OR
- [ ] The TOCTOU risk is documented with an explicit security note explaining the threat model
- [ ] `Path::canonicalize` replaced with `tokio::fs::canonicalize` (async) to avoid blocking the tokio thread
- [ ] Tests cover the symlink race scenario (symlink created after validation)

## Work Log

- 2026-03-06: Identified by security-sentinel review of feat/unified-task-engine
