---
module: mika-agent/tools/create_task
tags: [task-engine, configuration, guard, limits]
problem_type: enhancement
---

# Configurable per-session task creation limit

## Problem

Guard 5 in `create_task` hardcoded a limit of 5 agent-created tasks per session. Milestone dispatch with 7+ child issues hit this ceiling, forcing complex workarounds (deferring task creation to subsequent callbacks).

## Solution

Made the limit configurable via `max_agent_tasks_per_session` (config.toml or `MIKA_MAX_AGENT_TASKS_PER_SESSION` env var), defaulting to 25.

## Threading pattern

The limit flows: `Settings.max_agent_tasks_per_session` -> `ToolContext.max_tasks_per_session` -> used by `create_task` Guard 5. This follows the existing pattern where values are resolved once at ToolContext construction (like `brave_api_key`, `github_token`).

At all 3 ToolContext construction sites (conversation, silent, team) in `agent.rs`:
```rust
max_tasks_per_session: params.settings.map_or(25, |s| s.max_agent_tasks_per_session),
```

The `map_or(25, ...)` fallback covers cases where `settings` is `None` (test contexts, investigation panel).

## Key decisions

- **Field on ToolContext, not per-call config read:** Follows the established pattern. Avoids DB/file reads during tool execution.
- **Default 25 (not unbounded):** Covers the largest foreseeable milestone (18 tickets = 19 tasks with parent) while still catching runaway loops.
- **File backend:** Per-agent configurable via `config.toml` without env vars, with env var override for container deployments.
- **Tests use small limits (3-5):** Avoids 25-iteration test loops while still exercising the guard logic.
