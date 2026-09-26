---
status: complete
priority: p3
issue_id: "320"
tags: [code-review, reliability, tui]
dependencies: []
---

# No Timeout on Clipboard Subprocess Calls (xclip/wl-paste)

## Problem Statement

The Linux clipboard fallback functions `try_xclip_image()` and `try_wl_paste_image()` call external processes without a timeout. If xclip or wl-paste hangs (e.g., waiting for a display server), the TUI event loop blocks indefinitely.

## Findings

**Source:** security-sentinel

**Location:** `crates/mika-cli/src/tui/input.rs` — `try_xclip_image()` and `try_wl_paste_image()`

Both use `std::process::Command::new(...).output()` which blocks until the process exits.

## Proposed Solutions

### Option A: Add timeout via std::process with thread + wait_timeout
- Spawn the process and use a timeout mechanism
- **Pros:** Prevents indefinite hangs
- **Cons:** More complex than simple `.output()`
- **Effort:** Small-Medium
- **Risk:** Low

### Option B: Use tokio::process with timeout
- Switch to async process spawning with `tokio::time::timeout`
- **Pros:** Clean async integration
- **Cons:** Requires making clipboard functions async
- **Effort:** Medium
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/input.rs`
- **Note:** These functions are `#[cfg(target_os = "linux")]` only

## Acceptance Criteria

- [ ] Clipboard subprocess calls have a reasonable timeout (e.g., 2-3 seconds)
- [ ] Timeout returns None gracefully (no crash/hang)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #28 code review | Pre-existing code, not introduced by this PR |

## Resources

- PR: #28
