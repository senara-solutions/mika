---
title: Callback silent turns retain exec/http handler skills
category: architecture-patterns
date: 2026-04-14
issue: 567
tags: [agent-core, silent-mode, callbacks, skills, security]
---

# Callback silent turns retain exec/http handler skills

## Problem

When a long-running `run_claude_pilot` call crashed transiently (V8 startup error on 2026-04-14, sprint ticket mika#334), the callback turn fired with `failed: true` and self-dev's retry prompt instructed the agent to re-invoke `run_claude_pilot`. The tool call returned `Unknown tool: run_claude_pilot` twice. The task transitioned `in_progress → blocked` and the entire sprint stalled.

## Root Cause

`run_silent_inner()` unconditionally called `SkillRegistry::safe_always_on_skills()`, which filters out any skill with `Exec` or `Http` handlers. The filter was designed for fully autonomous triggers (heartbeat, reflection) where exec handlers should not run without user intent. It was never re-scoped when callback semantics were added.

A callback turn is fundamentally different from an autonomous trigger: it is the agent's continuation of a tool call it already made in conversation mode. The agent was already exposed to the full always-on skill set when it initiated the call, so the retry/continuation workflow must have access to the same tools.

## Solution

Split the silent-mode skill filter by trigger type:

```rust
// crates/mika-agent/src/agent.rs — run_silent_inner()
let matched = match &params.trigger {
    SilentTrigger::Callback { .. } => params.skills.callback_safe_skills(),
    SilentTrigger::Heartbeat
    | SilentTrigger::Reflection
    | SilentTrigger::Reminder { .. }
    | SilentTrigger::SkillRun { .. } => params.skills.safe_always_on_skills(),
};
```

New method on `SkillRegistry`:

```rust
// crates/mika-agent/src/skills/mod.rs
pub fn callback_safe_skills(&self) -> Vec<&SkillEntry> {
    self.skills
        .iter()
        .filter(|e| e.enabled && e.manifest.skill.always_on)
        .collect()
}
```

The exhaustive `match` (no wildcard) forces a compile error when a new `SilentTrigger` variant is added — preventing an accidental security regression where a new trigger silently inherits exec access.

## Security Reasoning

The filter was a **structural backstop** against exec handlers running in callback turns. That backstop is now relaxed for callbacks, but **four independent guards remain**:

1. **Dispatch-readiness guard (#525)** — `validate_dispatch_readiness()` rejects new long-running dispatches when the target task is not `pending`/`in_progress` OR an active callback child task already exists. A callback cannot spawn a new `run_claude_pilot` against the same task.
2. **`is_task_context: true`** in silent `ToolContext` blocks top-level `create_task` calls. A compromised callback result cannot make the agent fabricate a new task to sidestep into.
3. **Trust boundary** — callback `result` content is already wrapped in `<callback_result trust="untrusted">` framing with an explicit anti-instruction-following directive.
4. **`is_callback_turn` flag** now correctly propagates (`matches!(params.trigger, SilentTrigger::Callback { .. })`) to the silent `ToolContext`, giving future per-tool defense-in-depth (e.g., gating `shell_exec` on `!ctx.is_callback_turn`) a working hook.

Net capability delta vs. pre-fix: callback turns can now invoke the same exec/http handlers the agent already had in conversation mode. This is not new attack surface — it is restoration of parity with the originating call.

## Key Files

| File | Purpose |
|------|---------|
| `crates/mika-agent/src/skills/mod.rs` | `callback_safe_skills()` method + unit tests |
| `crates/mika-agent/src/agent.rs` | Trigger-aware match in `run_silent_inner()`; `is_callback_turn` flag propagation |
| `crates/mika-agent/CLAUDE.md` | Silent Mode Agent Loop section — split documented |
| `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` | Updated to reflect new backstop (dispatch-readiness guard, not skill filtering) |

## Prevention

When adding a new `SilentTrigger` variant:

- [ ] Decide whether it needs the permissive (`callback_safe_skills`) or restrictive (`safe_always_on_skills`) filter. The match in `run_silent_inner()` is exhaustive — the compiler will force you to make the choice.
- [ ] Default to `safe_always_on_skills`. Only `Callback` (or future variants that continue an agent-initiated tool call) should use the permissive path.
- [ ] Update the CLAUDE.md "Silent Mode Agent Loop" section to document the new variant's filter choice.

When adding a new always-on exec-handler skill:

- [ ] Remember it is now reachable from callback turns (not just conversation turns). Verify the skill is compatible with untrusted `result` input.
- [ ] If the skill must be off-limits in callback turns, consider adding a per-tool guard on `ctx.is_callback_turn` rather than relying on registry-level filtering.

## Related

- mika#334 — the blocked sprint ticket that surfaced the bug
- mika#525 — dispatch-readiness guard (the structural backstop that now carries callback loop-prevention)
- `project_claude_code_startup_crash` memory — transient V8 crash that triggers the retry path
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — companion doc (updated in same PR)
- `docs/solutions/architecture-patterns/skill-dependency-resolution-and-action-guard.md` — origin of `safe_always_on_skills()` and the exec/http filter

## Commits

- PR branch: `fix/567/callback-exec-handler-tools`
- Closes senara-solutions/mika#567
