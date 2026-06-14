---
status: complete
priority: p2
issue_id: "202"
tags: [code-review, architecture, ux]
dependencies: []
---

# DB-Only Commands Require API Key

## Problem Statement

`mika status`, `mika memory`, `mika reminders`, and `mika config` all call `init_db_only()` which calls `Settings::load()`. The `Settings` struct has `anthropic_api_key: String` as a required (non-optional) field. This means read-only commands that never touch the Claude API will fail if `MIKA_ANTHROPIC_API_KEY` is not set, with a confusing deserialization error.

## Findings

- **Source:** architecture-strategist (Finding 3), pattern-recognition-specialist (Finding 6)
- **Location:** `crates/mika-cli/src/init.rs:56-57`, `crates/mika-common/src/config.rs:9`
- **Evidence:** `init_db_only()` calls `Settings::load()` which requires all fields including `anthropic_api_key`. Error message says "Set MIKA_ANTHROPIC_API_KEY env var" for commands that don't need it.
- **Impact:** Users who want to inspect their database without an API key set get an opaque config error.

## Proposed Solutions

### Option 1: Make `anthropic_api_key` optional in Settings
- **Pros**: Clean fix, DB-only commands work without API key
- **Cons**: Requires validation deferred to `ClaudeClient::new()` or chat command
- **Effort**: Medium (touches Settings, ClaudeClient, all callers)
- **Risk**: Medium (changes a shared struct)

### Option 2: Create a lightweight `DbSettings` loader for read-only commands
- **Pros**: No changes to existing Settings struct
- **Cons**: Introduces a second settings type, some duplication
- **Effort**: Small
- **Risk**: Low

### Option 3: Default `anthropic_api_key` to empty string
- **Pros**: Minimal change
- **Cons**: Hides the missing key until chat time; empty string is a foot-gun
- **Effort**: Trivial
- **Risk**: Medium

## Recommended Action

Option 1 — make `anthropic_api_key` an `Option<String>` and validate at `ClaudeClient::new()`.

## Technical Details

- **Affected files:** `crates/mika-common/src/config.rs`, `crates/mika-cli/src/init.rs`

## Acceptance Criteria

- [ ] `mika status` works without `MIKA_ANTHROPIC_API_KEY` set
- [ ] `mika chat` still fails clearly if API key is missing
- [ ] No regression in mika-spirit API key validation

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
