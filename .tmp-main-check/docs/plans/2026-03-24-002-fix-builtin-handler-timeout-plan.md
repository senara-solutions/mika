---
title: "fix: Builtin handler timeout uses hardcoded 30s instead of skill timeout"
type: fix
status: active
date: 2026-03-24
---

# fix: Builtin handler timeout uses hardcoded 30s instead of skill timeout

## Overview

The `ToolHandler::Builtin` dispatch path in `agent.rs` hardcodes `TOOL_TIMEOUT_SECS` (30s) for builtin skill handlers, while exec and http handlers correctly use `dispatch.skill_timeout` (computed from `max_skill_timeout()`). This means builtin handlers like `run_gh` always time out at 30s regardless of the skill's configured `timeout_secs`.

## Problem Statement

In `crates/mika-agent/src/agent.rs` line 1215, the builtin handler branch uses:
```rust
std::time::Duration::from_secs(TOOL_TIMEOUT_SECS)
```

While the exec handler branch at line 1232 correctly uses:
```rust
dispatch.skill_timeout
```

This caused `run_gh(["pr", "diff", ...])` to time out at 30s on large PRs even though the github skill could declare a higher timeout.

## Proposed Solution

Two changes:

### 1. Fix builtin handler timeout (agent.rs:1213-1227)

Replace `TOOL_TIMEOUT_SECS` with `dispatch.skill_timeout` in the `ToolHandler::Builtin` branch. Add `timeout_secs` to the warn log for consistency.

### 2. Increase github skill timeout (templates/skills/github/skill.toml)

Change `timeout_secs = 30` to `timeout_secs = 120` to match `delegate_task` timeout. `gh pr diff` on large PRs (500+ lines) easily exceeds 30s.

## Acceptance Criteria

- [ ] Builtin handler timeout uses `dispatch.skill_timeout` not `TOOL_TIMEOUT_SECS`
- [ ] Warn log includes `timeout_secs` field
- [ ] Github skill timeout is 120s
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes
