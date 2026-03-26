---
title: "fix: remove work item write tools from default_tools()"
type: fix
status: completed
date: 2026-03-26
issue: "#278"
---

# fix: remove work item write tools from default_tools() — restrict to orchestrators only

## Problem

`create_work_item` and `update_work_item_status` are registered in `default_tools()`, making them available to **all** agent contexts — including delegate agents, team agents, and silent mode runs. Delegate/specialist agents (e.g. mika-qa) should never create or update work items; they receive `work_item_id` from the orchestrator via `delegate_task` and never need their own.

This caused mika-qa to fabricate work item labels before fetching PR metadata (senara-solutions/mika-skills#30). Prompt-level prohibition (senara-solutions/mika-skills#31) is insufficient — models like DeepSeek may ignore prompt rules. The fix must be structural: remove the tools from the registry so no model can call them.

## Proposed Solution

Move `create_work_item` and `update_work_item_status` from `default_tools()` to the conditional block inside `management_tools_if_needed()` (gated by `agents.len() > 1 || !teams.is_empty()`). This ensures only orchestrator agents have write access.

Keep `list_work_items` and `check_work_item` in `default_tools()` — read-only access is appropriate for all agents.

## Implementation

### Step 1: Modify `crates/mika-agent/src/tools/mod.rs`

**In `default_tools()` (lines 469-470):** Remove the two write tool registrations:

```rust
// REMOVE these two lines:
registry.register(Box::new(create_work_item::CreateWorkItemTool));
registry.register(Box::new(update_work_item_status::UpdateWorkItemStatusTool));

// KEEP these two lines:
registry.register(Box::new(list_work_items::ListWorkItemsTool));
registry.register(Box::new(check_work_item::CheckWorkItemTool));
```

**In `management_tools_if_needed()` conditional block (after line ~525):** Add the two write tools alongside `delegate_task`, `run_team`, etc.:

```rust
// Inside the `if agents.len() > 1 || !teams.is_empty()` block:
registry.register(Box::new(create_work_item::CreateWorkItemTool));
registry.register(Box::new(update_work_item_status::UpdateWorkItemStatusTool));
```

### Step 2: Update CLAUDE.md

Update the conventions section to reflect that:
- Work item **write** tools (`create_work_item`, `update_work_item_status`) are registered alongside management tools (orchestrator-only)
- Work item **read** tools (`list_work_items`, `check_work_item`) remain in `default_tools()`

### Step 3: Verify

- `cargo test` — all existing tests pass
- `cargo clippy` — no new warnings

## Acceptance Criteria

- [x] `create_work_item` not in `default_tools()` registry
- [x] `update_work_item_status` not in `default_tools()` registry
- [x] Both registered alongside management tools (orchestrator-only)
- [x] `list_work_items` and `check_work_item` remain in `default_tools()`
- [x] Existing tests pass (`cargo test`)

## Context

- Issue: #278
- Supersedes: senara-solutions/mika-skills#30 and senara-solutions/mika-skills#31 (prompt-level fix)
- Existing loop-prevention guards in `create_work_item.rs` stay for defense-in-depth
- Architecture pattern: [docs/solutions/architecture-patterns/callback-task-loop-prevention.md](../solutions/architecture-patterns/callback-task-loop-prevention.md)
- Integration doc: [docs/solutions/integration-issues/agent-team-management-tools-integration.md](../solutions/integration-issues/agent-team-management-tools-integration.md)

## Sources

- Related issue: #278
- Management tools pattern: `crates/mika-agent/src/tools/mod.rs` — `management_tools_if_needed()`
