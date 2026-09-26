---
title: fix: Callback turns cannot retry claude-pilot — exec handler tools filtered in silent mode
type: fix
status: active
date: 2026-04-14
issue: 567
---

# Callback turns cannot retry claude-pilot — exec handler tools filtered in silent mode

## Overview

When a long-running task (e.g. `run_claude_pilot`) fails transiently, the self-dev skill's retry logic fires in the callback turn — but `run_claude_pilot` is not registered as a tool. The callback turn is a `SilentTrigger::Callback` dispatched through `run_silent_inner()`, which unconditionally filters skills via `safe_always_on_skills()`, stripping any skill that declares an `Exec` or `Http` handler. The `claude-pilot` skill has `handler.type = "exec"`, so the retry attempts produce `Unknown tool: run_claude_pilot` and the work item transitions to `blocked`.

The fix: make `run_silent_inner()` trigger-aware when selecting skills. Callback turns (and ONLY callback turns) should retain exec/http handler skills because callbacks are a response to a tool the agent explicitly authorized, and the retry workflow is part of the intended design (the system prompt already tells the agent to "Follow the workflow defined by your active skills for this callback type"). All other silent triggers (`Heartbeat`, `Reflection`, `SkillRun`, `Reminder`) keep the current restricted filter.

## Problem Statement

**Symptom (2026-04-14, sprint ticket mika#334):**
- claude-pilot crashed on startup (SyntaxError, exit 1, 2 seconds). See `project_claude_code_startup_crash` memory.
- Callback delivered to mika-dev with `failed = true`.
- Self-dev's retry prompt told the agent to call `run_claude_pilot` again.
- Two `run_claude_pilot` tool calls → `Unknown tool: run_claude_pilot`.
- Work item `84f9ec29` transitioned `in_progress` → `blocked`. Sprint stalled.

**Root cause:** Overly broad filter. `safe_always_on_skills()` was written as a backstop against exec handlers running in autonomous heartbeat/reflection contexts (where tool calls are not user-authorized). The comment at `agent.rs:2064` applies the restriction to all silent triggers indiscriminately, but `SilentTrigger::Callback` is semantically different — it processes the result of a tool the agent already called.

## Proposed Solution

Introduce a `callback_safe_skills()` method on `SkillRegistry` that returns `always_on` skills **without** filtering exec/http handlers (still respecting `enabled` + `always_on`). Wire it into `run_silent_inner()` by matching on `params.trigger`:

```rust
let matched = match &params.trigger {
    SilentTrigger::Callback { .. } => params.skills.callback_safe_skills(),
    _ => params.skills.safe_always_on_skills(),
};
```

### Why this split (not a parameter or a global flag)

- **Named method makes intent obvious.** A future reader sees `callback_safe_skills()` and understands the contract: "this is the callback-context skill set; it permits exec handlers."
- **Two callers, two methods** beats one method with a boolean parameter (no need to scan call sites to understand which mode is active).
- **Heartbeat/Reflection/Reminder/SkillRun behavior is unchanged.** No regression risk for autonomous triggers.

### Why callbacks are safe to allow

1. **User/agent-authorized origin.** A callback is the result of a `run_claude_pilot` (or similar) tool call the agent explicitly made. The agent already passed through the conversation-mode skill set when it made that call.
2. **Dispatch-readiness guard (#525).** Work items must be `pending`/`in_progress` to dispatch a new long-running task — callbacks cannot silently re-dispatch to a different unrelated work item.
3. **Loop-prevention guards.** `callback-task-loop-prevention.md` already blocks callbacks from spawning `delegate_task`/`run_team`. The long-running skill path is also guarded (CLAUDE.md: "Loop prevention: callback turns cannot spawn new long-running tasks"). This plan does NOT weaken either guard.
4. **ToolContext flag.** `is_callback_turn: true` in the tool context already exists — individual tools can defense-in-depth against callback abuse if needed in the future.

### Why NOT extend to `Reminder` / `SkillRun` / `Heartbeat`

| Trigger     | Exec handler access | Why |
|-------------|---------------------|-----|
| `Callback`  | ✅ Allow             | Response to an agent-initiated tool call; retry requires same tool set |
| `Reminder`  | ❌ Deny              | User-created reminders should be predictable; arbitrary exec execution surprises the user |
| `Heartbeat` | ❌ Deny              | Fully autonomous; no user or agent intent behind the turn |
| `Reflection`| ❌ Deny              | Daily autonomous self-review; should not execute external commands |
| `SkillRun`  | ❌ Deny              | The single named skill is already executing; other exec-handler skills should not piggy-back |

## Affected Code

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/mod.rs` | Add `callback_safe_skills()` method (mirror of `safe_always_on_skills()` minus the exec/http filter) + unit tests |
| `crates/mika-agent/src/agent.rs` | Replace unconditional `safe_always_on_skills()` at line 2067 with a `match` on `params.trigger`. Update the comment at lines 2064-2066 to explain the split |
| `crates/mika-agent/src/skills/mod.rs` (tests) | Add test `test_callback_safe_skills_includes_exec_and_http` |
| `crates/mika-agent/tests/eval/` | Add eval test: callback trigger exposes `run_claude_pilot` (fake exec skill) as a tool |
| `crates/mika-agent/CLAUDE.md` | Update "Silent Mode Agent Loop" section to document the trigger-aware split |
| `docs/solutions/` | New solution doc capturing the fix rationale + security reasoning |

The `task_engine/dispatcher.rs` code paths are NOT modified — the dispatcher correctly constructs `SilentTrigger::Callback`; the bug is entirely in how `run_silent_inner()` consumes that trigger.

## Acceptance Criteria

### Functional Requirements

- [x] Callback turns (`SilentTrigger::Callback`) include `always_on` skills even when they have exec/http handlers
- [x] Self-dev retry logic can successfully call `run_claude_pilot` from a callback turn after a transient failure
- [x] Heartbeat turns exclude exec/http handler skills (regression check)
- [x] Reflection, Reminder, SkillRun turns exclude exec/http handler skills (regression check)
- [x] `callback_safe_skills()` still honors `enabled = true` and `always_on = true` — only the handler-type filter is relaxed

### Non-Functional Requirements

- [x] No change to conversation-mode skill resolution (full `match_skills()` path remains untouched)
- [x] Loop-prevention guards for callback turns remain intact (no new long-running task dispatch from callback)
- [x] No new dependencies

### Quality Gates

- [x] New unit tests in `skills/mod.rs` for `callback_safe_skills()`
- [x] Updated tests in `skills/mod.rs` to confirm `safe_always_on_skills()` behavior is unchanged
- [x] New eval test in `tests/eval/` exercising `SilentTrigger::Callback` with a fake exec-handler skill, asserting the tool appears in `ToolRegistry`
- [x] `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` all green

## Implementation Plan

### Step 1 — Add `callback_safe_skills()` to `SkillRegistry`

`crates/mika-agent/src/skills/mod.rs`:

```rust
/// Returns enabled, always-on skills — including those with exec/http handlers.
///
/// Used exclusively for `SilentTrigger::Callback` turns, where the agent is
/// processing the result of a tool it explicitly authorized. Callbacks are
/// the expected continuation of a conversation-mode tool call, so the skill
/// set that was available to the originating call must remain available to
/// retry/continue the workflow.
///
/// Compare with `safe_always_on_skills()`, which strips exec/http handlers
/// for fully autonomous triggers (Heartbeat, Reflection, Reminder, SkillRun).
pub fn callback_safe_skills(&self) -> Vec<&SkillEntry> {
    self.skills
        .iter()
        .filter(|e| e.enabled && e.manifest.skill.always_on)
        .collect()
}
```

### Step 2 — Wire trigger-aware matching in `run_silent_inner()`

`crates/mika-agent/src/agent.rs` around line 2064:

```rust
// Match skills based on trigger type:
// - Callback: user/agent-authorized retry — exec/http handlers allowed
// - Heartbeat/Reflection/Reminder/SkillRun: autonomous — exec/http handlers stripped
// No per-skill LLM override in silent mode (#463): both paths return only
// AlwaysOn entries, and resolve_skill_llm_override filters to Keyword only.
let matched = match &params.trigger {
    SilentTrigger::Callback { .. } => params.skills.callback_safe_skills(),
    _ => params.skills.safe_always_on_skills(),
};
```

### Step 3 — Unit tests in `skills/mod.rs`

Add alongside the existing `test_safe_always_on_skills_*` tests:

```rust
#[test]
fn test_callback_safe_skills_includes_exec_and_http() {
    // Registry with: prompt-only always_on, exec-handler always_on, http-handler always_on,
    // non-always-on exec-handler, disabled exec-handler.
    // Assert callback_safe_skills() returns the three always_on + enabled, regardless of handler type.
    // Assert safe_always_on_skills() still returns only the prompt-only one.
}

#[test]
fn test_callback_safe_skills_respects_enabled_and_always_on() {
    // Assert disabled skills are excluded even for callbacks.
    // Assert non-always_on skills are excluded even for callbacks.
}
```

### Step 4 — Eval test for callback trigger tool availability

`crates/mika-agent/tests/eval/` — add a test that:

1. Builds an `EvalHarness` with a fake `always_on` exec-handler skill exposing tool `fake_retry_tool`
2. Dispatches a `SilentTrigger::Callback { failed: true, .. }` via the harness's silent-agent entry point
3. Asserts `fake_retry_tool` appears in the tool definitions passed to the mock LLM provider
4. Contrast: dispatch a `SilentTrigger::Heartbeat` and assert `fake_retry_tool` does NOT appear

### Step 5 — Documentation updates

- **`crates/mika-agent/CLAUDE.md`** — in "Silent Mode Agent Loop" section, update the line referencing `safe_always_on_skills()` to describe the trigger-aware split:
  > "Heartbeat, Reflection, Reminder, and SkillRun modes use `safe_always_on_skills()` which filters out exec/http-handler skills. Callback mode uses `callback_safe_skills()` which preserves exec/http handlers so the agent can retry or continue the workflow it originally authorized."
- **`docs/solutions/callback-exec-handler-tool-availability.md`** — new solution doc (created by `/ce:compound`).

### Step 6 — Non-changes (for review clarity)

The following are **out of scope** and must not change:
- `run_silent_inner()` step-limit logic (`SilentTrigger::max_steps()`)
- Callback loop-prevention guards in tool handlers
- Dispatch-readiness guard for long-running skills (#525)
- Conversation-mode skill matching
- System prompt framing (`build_callback_trigger_context`)

## System-Wide Impact

### Interaction Graph

1. `TaskDispatcher::dispatch_resume_agent()` constructs `SilentTrigger::Callback` (dispatcher.rs:302)
2. → `run_silent_agent(params)` → `run_silent_inner(params)` (agent.rs:1896)
3. → **NEW:** `match params.trigger` selects `callback_safe_skills()` (agent.rs:2067)
4. → `inject_skills_and_resolve_tools()` (agent.rs:2724) — now receives exec-handler skill, injects its tools
5. → LLM call → callback retry succeeds
6. → Exec handler dispatch path (for the retried `run_claude_pilot`) → existing dispatch-readiness guard (#525) + loop-prevention guard apply unchanged

### Error & Failure Propagation

No new error paths. The failure mode being fixed — `Unknown tool: run_claude_pilot` — disappears because the tool is now registered for this turn type. Existing error paths from exec handler failures continue to flow through callback framing + `failed: true` semantics.

### State Lifecycle Risks

- **Callback re-dispatch loop:** Not possible. Work-item guards (#525) require `pending`/`in_progress` status and no active callback child task. A callback cannot spawn a second `run_claude_pilot` against the same work item if one is already in flight.
- **Work item status drift:** The retry path uses the existing `run_claude_pilot` tool, which uses the same dispatch/state transition code as the original call. No new state transitions introduced.

### API Surface Parity

- `SkillRegistry::safe_always_on_skills()` remains unchanged
- New `SkillRegistry::callback_safe_skills()` is additive
- No public API outside the agent crate is affected

### Integration Test Scenarios

1. **Callback with exec retry succeeds:** simulate `run_claude_pilot` callback with `failed=true`, confirm retry tool call reaches dispatcher
2. **Heartbeat cannot call exec tool:** simulate heartbeat, confirm exec-handler skill's tool is absent from tool registry
3. **Disabled exec skill excluded:** registry with disabled exec-handler skill, callback trigger — skill must still be excluded
4. **Non-always-on exec skill excluded:** registry with `always_on = false` exec skill, callback trigger — skill must still be excluded
5. **Multiple always_on skills:** callback trigger with mixed prompt-only + exec + http skills, all three appear in `matched`

## Dependencies & Risks

### Dependencies

None. The `SilentTrigger::Callback` variant already carries all information needed; no schema migrations, no new env vars.

### Risks

- **Risk:** Callback turn executes an unexpected exec handler from an unrelated always_on skill.
  **Mitigation:** Existing conversation-mode skill set already included all always_on skills during the originating tool call. The retry turn gets the same skill set — nothing new is reachable. Agent already had this "capability surface" by construction.
- **Risk:** Future contributor adds a dangerous always_on exec skill and forgets callback mode is now permissive.
  **Mitigation:** `callback_safe_skills()` doc comment explicitly names the contract. New solution doc records the security reasoning. CLAUDE.md updated.

## Success Metrics

- mika#334-class sprint failures (callback retry fails with `Unknown tool`) drop to zero
- `project_claude_code_startup_crash` memory note can reference this fix as the recovery mechanism
- No regression in tool-filtering tests for Heartbeat/Reflection/Reminder/SkillRun triggers

## Sources & References

### Origin

- GitHub Issue: senara-solutions/mika#567
- Related incident memory: `project_claude_code_startup_crash` (transient V8 startup crash triggers the callback retry path)

### Internal References

- `crates/mika-agent/src/agent.rs:2064-2067` — current unconditional filter
- `crates/mika-agent/src/agent.rs:1782-1822` — `SilentTrigger` enum
- `crates/mika-agent/src/skills/mod.rs:311-327` — `safe_always_on_skills()`
- `crates/mika-agent/src/task_engine/dispatcher.rs:273-409` — callback dispatch path
- `crates/mika-agent/CLAUDE.md` — "Silent Mode Agent Loop" section

### Related Solutions

- `skill-dependency-resolution-and-action-guard.md` — origin of `safe_always_on_skills()` and the exec/http filter
- `callback-task-loop-prevention.md` — existing loop-prevention guards for callbacks
- `generic-callback-framing-parent-task-id.md` — callback trigger framing

### Related Work

- PR #463 — per-skill LLM override keyword filter (comment referenced in current code)
- PR #525 — dispatch-readiness guard (ensures retry cannot corrupt work-item state)
- PR #528 — webhook deferral queue (complementary callback-path hardening)
