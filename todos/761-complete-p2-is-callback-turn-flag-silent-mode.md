---
status: pending
priority: p2
issue_id: 761
tags: [code-review, security, agent-core]
dependencies: []
---

# `is_callback_turn` hardcoded false in silent callback turns — breaks per-tool defense-in-depth

## Problem Statement

PR #567 opens exec/http handler skills to `SilentTrigger::Callback` turns. The plan's threat-model argument (safe because per-tool defense-in-depth via `ctx.is_callback_turn` is available) is undermined by `crates/mika-agent/src/agent.rs:2136`, which hardcodes `is_callback_turn: false` in `run_silent_inner()` with a comment saying it's "only meaningful in the Conversation mode path."

In silent callback turns, `params.trigger` is `SilentTrigger::Callback { .. }`, but the tool context still receives `is_callback_turn = false`. Any future per-tool hardening (e.g., gating `shell_exec` on `!ctx.is_callback_turn`) will not fire for the exact code path the fix opens up.

## Findings

- **security-sentinel review** flagged this explicitly as the one gap worth closing before merge.
- **Evidence:** `crates/mika-agent/src/agent.rs:2136` — comment + hardcoded `false`.

## Proposed Solution

Change the `is_callback_turn` field in the silent-mode `ToolContext` construction to match the trigger:

```rust
is_callback_turn: matches!(params.trigger, SilentTrigger::Callback { .. }),
```

Zero behavioral change today (no tool reads the flag in silent mode yet), but gives future defenses a working hook. Also update the nearby comment.

**Pros:** Cheap (1 line + comment), aligns runtime state with intent, enables future hardening.
**Cons:** None.
**Effort:** Small.
**Risk:** None — no consumer reads the flag in silent mode today.

## Acceptance Criteria

- [ ] `ctx.is_callback_turn` is `true` when `SilentTrigger::Callback { .. }`, `false` otherwise
- [ ] Comment at `agent.rs:2136` updated to reflect the new behavior
- [ ] `cargo clippy -- -D warnings` and full test suite pass

## Resources

- PR branch: `fix/567/callback-exec-handler-tools`
- Issue: senara-solutions/mika#567
- File: `crates/mika-agent/src/agent.rs:2125-2140`
