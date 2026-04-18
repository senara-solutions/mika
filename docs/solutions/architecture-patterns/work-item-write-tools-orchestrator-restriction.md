---
title: "Restrict task write tools to orchestrator agents"
category: architecture-patterns
date: 2026-03-26
tags: [tools, registration, work-items, delegation, structural-enforcement]
issue: "#278"
---

# Restrict task write tools to orchestrator agents

## Problem

Delegate and specialist agents (e.g. mika-qa) could call `create_task` and `update_task_status` because these tools were registered in `default_tools()`, which is available to all agent contexts. This caused mika-qa to fabricate task labels before fetching PR metadata. Prompt-level prohibition was insufficient — models like DeepSeek may ignore prompt rules.

## Root Cause

Task creation and status updates are orchestration concerns. `create_task` and `update_task_status` were registered in `default_tools()` (available to all agents) rather than in `management_tools_if_needed()` (orchestrator-only). The existing pattern of structural enforcement (delegates get `default_tools()` only, no management tools) was not applied to task write tools.

## Solution

Move `create_task` and `update_task_status` from `default_tools()` to the conditional block in `management_tools_if_needed()` (gated by `agents.len() > 1 || !teams.is_empty()`). Keep read-only tools (`list_tasks`, `check_task`) in `default_tools()`.

```rust
// In default_tools() — read-only tools remain:
registry.register(Box::new(list_tasks::ListWorkItemsTool));
registry.register(Box::new(check_task::CheckWorkItemTool));

// In management_tools_if_needed() conditional block — write tools:
tools.push(Box::new(create_task::CreateWorkItemTool));
tools.push(Box::new(update_task_status::UpdateWorkItemStatusTool));
```

Key design decisions:
- **Conditional block (not always-on):** Tasks serve the delegation workflow. In single-agent setups without teams, there is no delegation, so task creation is unnecessary.
- **Read tools stay universal:** Delegates and silent agents legitimately need to read task state (e.g., callback handlers, heartbeat health monitor).
- **Defense in depth preserved:** The five loop-prevention guards in `create_task.rs` remain as a safety net even though the tools are now structurally restricted.

## Prevention

When adding new write tools that are orchestrator-specific:
1. Register them in `management_tools_if_needed()` (conditional block), not `default_tools()`
2. Follow the established pattern: structural enforcement over prompt-based prohibition
3. Keep corresponding read-only tools in `default_tools()` for visibility
4. Update `docs/architecture.md` tool tables to match registration location

## Related

- [callback-task-loop-prevention.md](callback-task-loop-prevention.md) — establishes the structural enforcement pattern
- [delegation-work-item-guard-enforcement.md](delegation-work-item-guard-enforcement.md) — task guard for delegate_task
- [docs/solutions/integration-issues/agent-team-management-tools-integration.md](../integration-issues/agent-team-management-tools-integration.md) — DRY registration rule for management_tools_if_needed()
- Issue: #278
