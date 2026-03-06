---
status: pending
priority: p2
issue_id: "502"
tags: [code-review, architecture, correctness, agent-native]
dependencies: []
---

# `dispatch_skill_by_name` Uses `SilentTrigger::Heartbeat` — Wrong Prompt Context and Tool Set

## Problem Statement

`dispatch_skill_by_name` passes `SilentTrigger::Heartbeat` to `SilentAgentParams` for arbitrary skill task runs. This means an agent-created `run_skill` task with any `skill_name` runs under the heartbeat system prompt ("Review the user's commitments, upcoming events...") and uses `safe_always_on_skills()` which filters out exec/http-handler skills. A scheduled skill task that needs `tmux` or `shell-exec` will silently receive an empty tool set and fail.

## Findings

- **Source**: architecture-strategist (F-3 Medium), security-sentinel (F-3 Medium), performance-oracle (F-3), patterns-reviewer (F-3 Substantive), simplicity-reviewer (F-2)
- **Location**: `crates/mika-agent/src/task_engine/dispatcher.rs:162`

```rust
let params = SilentAgentParams {
    ...,
    trigger: SilentTrigger::Heartbeat, // use heartbeat mode for skill runs
    ...
};
```

The comment acknowledges the hack. Concrete consequences:
1. Agent receives heartbeat context prompt — semantically wrong for a skill task
2. Conversation recorded with `channel_type = "heartbeat"` — misleading logs
3. `safe_always_on_skills()` excludes exec/http-handler skills — skill tasks that need `tmux` or `shell-exec` silently get empty tool sets and fail without error

The `skill_name` parameter is only used for logging (line 155), not for prompt construction or tool selection.

## Proposed Solutions

### Option A: Add `SilentTrigger::SkillRun` variant (Recommended)

```rust
pub enum SilentTrigger {
    Heartbeat,
    Reflection,
    Callback { label: String, result: String },
    SkillRun { skill_name: String },
}
```

In `run_silent_inner` match arm:
```rust
SilentTrigger::SkillRun { skill_name } => {
    format!("Run the '{skill_name}' skill and use send_message if there is output for the user.")
}
```

In `dispatcher.rs`:
```rust
trigger: SilentTrigger::SkillRun { skill_name: skill_name.to_string() },
```

Decide explicitly whether skill runs use `safe_always_on_skills()` (secure) or full registry (powerful but exposes exec handlers in background). Recommended: use the full skill registry since the user explicitly scheduled the skill by name.

- **Effort**: Small | **Risk**: Low

### Option B: Reject `run_skill` with `skill_name` config until properly implemented

Add validation in `dispatch_skill_by_name` that returns an error (marking the task `failed`) rather than running silently with wrong context.

- **Effort**: Tiny | **Risk**: None (breaks existing capability)

## Acceptance Criteria

- [ ] `SilentTrigger::SkillRun { skill_name: String }` variant exists
- [ ] `run_silent_inner` uses the skill name to build an appropriate context prompt
- [ ] `dispatch_skill_by_name` uses `SilentTrigger::SkillRun`, not `Heartbeat`
- [ ] Conversation `channel_type` for skill runs is `"skill_task"`, not `"heartbeat"`
- [ ] Skill runs can access the full tool registry (exec/http-handler skills available)

## Work Log

- 2026-03-06: Identified by multiple review agents for feat/unified-task-engine
