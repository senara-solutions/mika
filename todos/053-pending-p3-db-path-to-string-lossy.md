---
status: pending
priority: p3
issue_id: "053"
tags: [code-review, security, quality, rust-v2]
dependencies: []
---

# db_path Uses to_string_lossy Which Can Silently Corrupt Paths

## Problem Statement
In config.rs, `db_path` is constructed using `to_string_lossy().to_string()`, which silently replaces non-UTF-8 bytes with the Unicode replacement character. This could cause the database to be opened at an unexpected path.

**Location:** `crates/mika-common/src/config.rs:89-93`

**Reported by:** security-sentinel

## Proposed Solutions
Change `db_path` to `PathBuf` instead of `String`, or use `to_str()` with an explicit error.
- **Effort:** Small

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
