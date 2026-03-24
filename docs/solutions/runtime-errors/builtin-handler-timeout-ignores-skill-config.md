---
title: "Builtin handler timeout ignores skill config"
category: runtime-errors
date: 2026-03-24
tags: [timeout, builtin-handler, skill-timeout, run_gh, agent-loop]
modules: [agent, skills]
---

# Builtin handler timeout ignores skill config

## Problem

`run_gh(["pr", "diff", ...])` always timed out at 30s on large PRs, even when the github skill could declare a higher timeout. mika-qa's PR review for a +1,433 line diff timed out, and (separately) the qa-review prompt had no guardrail for this failure, leading to a fabricated review.

## Root Cause

The `ToolHandler::Builtin` dispatch path in `agent.rs` (line 1215) hardcoded `TOOL_TIMEOUT_SECS` (30s):

```rust
std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
```

Meanwhile, the exec handler path (line 1232) and http handler path correctly used `dispatch.skill_timeout`, which is computed from `max_skill_timeout()` — the maximum `timeout_secs` across all matched skills for the turn. The builtin handler was the only dispatch path that didn't respect this.

The timeout plumbing (`max_skill_timeout()` → `dispatch.skill_timeout`) already existed and was well-tested (4 unit tests). The bug was simply that the builtin branch didn't use it.

## Solution

1. **`agent.rs`**: Replace `TOOL_TIMEOUT_SECS` with `dispatch.skill_timeout` in the `ToolHandler::Builtin` branch. Added `timeout_secs` to the `warn!` log for consistency with other timeout paths.

2. **`templates/skills/github/skill.toml`**: Increase `timeout_secs` from 30 to 120. This matches the `delegate_task` timeout (120s) and gives `gh pr diff` enough headroom for large PRs.

## Prevention

- **Consistency check**: When adding a new dispatch path for tool execution, verify it uses the same timeout source as existing paths. The three handler types (builtin, exec, http) should all use `dispatch.skill_timeout`.
- **The timeout chain**: `skill.toml timeout_secs` → `SkillEntry.effective_timeout(provider, model)` → `max_skill_timeout()` → `dispatch.skill_timeout` → `tokio::time::timeout()`. Every dispatch path must end at the same place.
