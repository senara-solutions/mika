---
status: complete
priority: p3
issue_id: 538
tags: [code-review, duplication, teams]
dependencies: []
---

# TeamEngine::new_for_resume Duplicates new() (~55 lines)

## Problem Statement

`new_for_resume` and `new` share nearly identical initialization logic: building agents map, creating Claude client, building tool registry. Only differences: `new_for_resume` accepts a `TeamRun` instead of constructing one and skips `create_dir_all`.

**Severity:** P3 — Maintenance burden, changes to init must be duplicated.

## Findings

- `crates/mika-agent/src/teams/engine.rs:63-140` — `new()`
- `crates/mika-agent/src/teams/engine.rs:143-200` — `new_for_resume()` (near-copy)

## Proposed Solutions

1. **Extract shared `init_resources()` helper**
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Common initialization logic extracted to shared helper
- [ ] Both constructors use the shared helper
