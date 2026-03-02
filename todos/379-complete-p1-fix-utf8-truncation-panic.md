---
status: complete
priority: p1
issue_id: 379
tags: [code-review, security, bug]
dependencies: []
---

# Fix UTF-8 truncation panic in get_team_status deliverable preview

## Problem Statement

`get_team_status.rs:112-116` sliced the deliverable string at byte index 500 using `&deliverable[..500]`. If byte 500 falls in the middle of a multi-byte UTF-8 character (CJK, emoji, accented), Rust panics with `byte index is not a char boundary`.

## Findings

- **Source:** Security Sentinel, Code Simplicity Reviewer
- **File:** `crates/mika-agent/src/tools/get_team_status.rs:112-116`
- **Severity:** Medium - deterministic crash on multi-byte content

## Resolution

Used `is_char_boundary()` to walk back to a valid boundary before slicing, matching the existing `truncate_summary` pattern in `agent.rs`.

## Work Log

- 2026-03-02: Fixed by walking back to valid UTF-8 char boundary before slicing
