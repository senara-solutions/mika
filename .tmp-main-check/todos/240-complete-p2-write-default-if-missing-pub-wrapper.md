---
status: complete
priority: p2
issue_id: "240"
tags: [code-review, code-smell]
dependencies: []
---

# `write_default_if_missing_pub` is an unnecessary wrapper

## Problem Statement

`write_default_if_missing_pub` at `home.rs:227-229` is a `_pub` suffixed wrapper around a private function with identical signature. The original function should simply be made `pub`.

## Findings

- **Source:** Architecture Strategist, Pattern Recognition, Code Simplicity
- **File:** `crates/mika-common/src/home.rs:227-229`

## Proposed Solutions

Make `write_default_if_missing` public directly. Update all callers to use the original name.

## Acceptance Criteria

- [ ] `write_default_if_missing` is `pub`
- [ ] `write_default_if_missing_pub` wrapper removed
- [ ] All callers updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | |
