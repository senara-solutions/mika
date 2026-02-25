---
status: pending
priority: p2
issue_id: 259
tags: [code-review, quality, simplification]
dependencies: []
---

# Remove Dead Config Fields (YAGNI Violations)

## Problem Statement

Three YAGNI violations exist in the teams data structures: (1) `TeamFlow.steps` is never used to control execution (the flow is hardcoded), only displayed cosmetically. (2) `TeamRun.current_step` is set but never read or displayed. (3) `TeamRunSummary` is redundant -- it fully deserializes `TeamRun` then copies 6 fields into a new struct.

## Findings

- **Files:**
  - `crates/mika-common/src/team.rs` -- `steps` field on `TeamFlow`
  - `crates/mika-agent/src/teams/types.rs` -- `current_step` on `TeamRun`, `TeamRunSummary` struct
  - `crates/mika-agent/src/teams/engine.rs` -- sets `current_step` but nothing reads it
  - `crates/mika-agent/src/teams/history.rs` -- `TeamRunSummary` conversion logic

### Finding 1: `TeamFlow.steps` is cosmetic only
- The `steps` field is defined in team TOML configs and stored on `TeamFlow`
- The engine never iterates over `steps` to determine execution order
- Execution flow is hardcoded: plan -> execute -> synthesize
- Steps are only used for display in `team list` output

### Finding 2: `TeamRun.current_step` is write-only
- `current_step` is set during execution (e.g., "planning", "executing", "synthesizing")
- No code reads `current_step` for display, progress reporting, or logic
- It is serialized to JSON in the run history but never queried or shown

### Finding 3: `TeamRunSummary` is redundant
- `TeamRunSummary` contains a subset of `TeamRun` fields (6 of ~10)
- `list_runs()` deserializes full `TeamRun` from JSON, then maps to `TeamRunSummary`
- The summary struct adds no value over just using `TeamRun` directly

## Proposed Solutions

1. **Remove `steps` from `TeamFlow`:** Delete the field and remove it from TOML parsing. Document the fixed execution flow (plan -> execute -> synthesize) in a comment instead.

2. **Remove `current_step` from `TeamRun`:** Delete the field and all assignments to it.

3. **Delete `TeamRunSummary`:** Have `list_runs()` return `Vec<TeamRun>` directly. Callers that only need summary fields can access them on `TeamRun`.

Estimated impact: ~45 lines of code removed, simpler data model.

## Technical Details

- Need to verify that removing `steps` from TOML parsing does not break existing team definition files (use `#[serde(default)]` during migration or remove from all TOML files)
- `TeamRun` serialization format in the database may need a migration consideration, though removing fields from the struct should be backward-compatible with `serde(default)`
- The `list_runs` return type change affects CLI display code in the TUI

## Acceptance Criteria

- [ ] `steps` field removed from `TeamFlow` and team TOML files
- [ ] `current_step` field removed from `TeamRun` and all assignment sites
- [ ] `TeamRunSummary` struct deleted; `list_runs()` returns `Vec<TeamRun>`
- [ ] Code compiles cleanly with no warnings
- [ ] All existing tests pass
- [ ] Existing team run history remains readable (backward-compatible deserialization)

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
