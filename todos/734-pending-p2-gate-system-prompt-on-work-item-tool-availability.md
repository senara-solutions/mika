---
id: 734
status: pending
priority: p2
tags: [code-review, architecture, prompt]
---

# Gate system prompt sections on work item tool availability

## Problem Statement

The system prompt in `crates/mika-agent/src/prompt.rs` (lines ~407-427) unconditionally includes guidance for `create_work_item`, `update_work_item_status`, and the delegation rule. After moving work item write tools to the conditional block in `management_tools_if_needed()`, single-agent setups and delegate agents see prompt guidance for tools that are not registered, which could cause hallucinated tool calls.

This is a **pre-existing issue** — the same prompt also unconditionally references `delegate_task` which is already conditionally registered. The fix for #278 does not make this worse; it makes the work item tools consistent with how `delegate_task` already works.

## Findings

- `prompt.rs` lines ~407-427 unconditionally emit work item and delegation guidance
- The conditional-investigation-tool-registration pattern (docs/solutions/) establishes the convention: "when a tool is conditionally registered, the system prompt must also be conditional"
- Heartbeat health injection (`task_health_awareness`) assumes `update_work_item_status` is available for all agents
- The `has_github_tool` flag pattern in `server/investigate.rs` shows how to gate prompt sections

## Proposed Solutions

### Option A: Pass tool availability flags to prompt builder
- Add `has_management_tools: bool` parameter to `build_system_prompt()`
- Gate work item write guidance and delegation sections on this flag
- **Pros**: Clean, follows `has_github_tool` precedent
- **Cons**: Adds a parameter to the prompt builder
- **Effort**: Small
- **Risk**: Low

### Option B: Accept tool list and derive sections
- Pass the full `ToolRegistry` (or tool name list) to the prompt builder
- Auto-detect which sections to include based on registered tools
- **Pros**: More flexible, scales to future conditional tools
- **Cons**: More complex, may be YAGNI
- **Effort**: Medium
- **Risk**: Low

## Technical Details

- **Affected files**: `crates/mika-agent/src/prompt.rs`
- **Related**: `crates/mika-agent/src/agent.rs` (heartbeat injection)

## Acceptance Criteria

- [ ] System prompt only references `create_work_item`/`update_work_item_status` when tools are registered
- [ ] Heartbeat anomaly injection gated on tool availability
- [ ] `delegate_task` prompt section also gated (pre-existing)

## Resources

- PR: #278
- Pattern: docs/solutions/architecture-patterns/conditional-investigation-tool-registration.md
- Related: docs/solutions/integration-issues/agent-team-management-tools-integration.md
