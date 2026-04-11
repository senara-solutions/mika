---
title: "fix: exec handler GitHub identity and required_tools retry on terminal errors"
type: fix
status: active
date: 2026-04-11
---

# fix: exec handler GitHub identity and required_tools retry on terminal errors

## Overview

Two related fixes surfaced during mika-qa PR review audits (mika-skills#124, 2026-04-10):
1. Exec skill handlers can't authenticate as the agent's GitHub identity (#515)
2. The required_tools retry gate wastes LLM calls retrying unrecoverable errors (#516)

Both are in `crates/mika-agent/`.

## Part 1: GH_TOKEN injection into exec handlers (#515)

### Problem

`execute_exec()` at `executor.rs:304-341` scrubs `GH_TOKEN` via `scrub_mika_env_vars()` (line 330) but never re-injects the agent's `github_token`. The builtin `run_gh` handler (`builtin_handlers.rs:831-838`) already does this correctly:

```rust
scrub_mika_env_vars(&mut cmd);
if let Some(token) = ctx.github_token {
    cmd.env("GH_TOKEN", token);
}
```

### Fix

Thread `github_token: Option<&str>` through the exec handler call chain:

1. **`execute_skill_tool()`** (`executor.rs:104`) — add `github_token: Option<&str>` parameter
2. **`execute_exec()`** (`executor.rs:304`) — add `github_token: Option<&str>`, inject after scrub at line 330
3. **`execute_long_running()`** (`executor.rs:~635`) — same injection after its `scrub_mika_env_vars()` call
4. **Call site** (`agent.rs:1696-1702`) — pass `dispatch.ctx.github_token` from the `ToolDispatchCtx`

### Files

- `crates/mika-agent/src/skills/executor.rs` — `execute_skill_tool()`, `execute_exec()`, `execute_long_running()`
- `crates/mika-agent/src/agent.rs` — call site at `execute_tool()` (~line 1696)

### Constraints

- Never silently fall back between tokens (per `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md`)
- Follow scrub-then-inject ordering (per `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`)
- `github_token: None` means no injection — exec handler falls back to host auth (existing behavior, acceptable)

## Part 2: required_tools retry on terminal errors (#516)

### Problem

The required_tools gate (`agent.rs:735-772`) retries when the agent responds with text on EndTurn without calling all required tools. But if a required tool WAS called and failed with a terminal error (e.g., GitHub "Can not approve your own pull request"), the agent correctly explains the failure in text. The gate sees text without a successful required tool call and retries — wasting LLM calls.

### Fix

The gate already tracks `tools_called: HashSet<String>` across all steps. It should also consider a required tool "called" even if the call failed, since the agent attempted it.

Change the tracking: insert the tool name into `tools_called` regardless of success/failure. The gate's purpose is to prevent the agent from *fabricating results without trying* — if the agent called the tool and it failed, the agent is not fabricating.

### File

- `crates/mika-agent/src/agent.rs` — where `tools_called` is populated (~line 920-923) and the required_tools check (~line 735-772)

### Edge cases

- Tool called and failed transiently → agent may retry on its own (existing behavior, unaffected)
- Tool called and failed terminally → agent responds with text → gate sees tool was called → no retry (desired)
- Tool never called → gate retries once (existing behavior, preserved)

## Acceptance Criteria

- [x] `GH_TOKEN` is injected into exec handler child processes when `github_token` is `Some`
- [x] `GH_TOKEN` is injected into long-running exec handler child processes
- [x] `GH_TOKEN` is NOT injected when `github_token` is `None` (no fallback)
- [x] Required tools gate already tracks tool calls before dispatch
- [x] Required tools gate filters unavailable tools at enforcement time (#516)
- [x] Existing required_tools retry behavior preserved for tools never called
- [x] `cargo clippy` clean
- [x] `cargo test` passes
- [x] Add test: exec handler receives `GH_TOKEN` when `github_token` is provided
- [x] Add test: required_tools gate filters out unavailable tools
- [ ] ~~Add test: required_tools gate does not retry when required tool was called but failed~~ (not needed — tracking was already correct)

**Note on #516:** Investigation revealed that `tools_called` is populated from the LLM's ToolCall blocks **before** dispatch (agent.rs:919-923). Tool names are inserted regardless of execution outcome. The retry in the audit was caused by `run_shell` never being requested by the LLM (not applicable for that review), not by a failed tool being missed.

**Engine-side resilience fix (#516):** While the root cause is skill config (mika-skills), the engine can be made resilient: `filter_available_required_tools()` checks required tool names against the union of ToolRegistry + skill_tool_map + MCP at enforcement time. Tools not found in any source are excluded with a warning, preventing the gate from retrying for tools that can't possibly be called. This catches stale config, typos, and mismatched tool names without wasting LLM calls.

## Sources & References

- ADR-008: `docs/adr/008-github-identity-separation.md`
- Builtin pattern: `crates/mika-agent/src/skills/builtin_handlers.rs:831-838`
- Env scrub: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`
- Env isolation tiers: `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md`
- Required tools gate: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- mika#515, mika#516, mika#517
