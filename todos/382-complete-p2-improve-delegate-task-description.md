---
status: complete
priority: p2
issue_id: 382
tags: [code-review, agent-native, ux]
dependencies: []
---

# Improve delegate_task tool description to clarify limitations

## Problem Statement

The `delegate_task` description says "WITHOUT management tools (cannot delegate further)" but understates the limitations. The delegate has NO management tools at all (no `list_agents`, `list_teams`, `run_team`, etc.) and no MCP server access. If a user says "ask researcher to check which teams are available," the agent might delegate that task and it would fail.

## Findings

- **Source:** Agent-Native Reviewer (both instances)
- **File:** `crates/mika-agent/src/tools/delegate_task.rs:26`
- Delegate also has `mcp_manager: None` at line 132, silently removing MCP capabilities

## Proposed Solutions

### Option 1: Expand description (Recommended)
Change to: "Delegate a task to another agent and get their response. The delegate agent runs with its own personality, memory, and skills. It has NO management tools (cannot list agents, run teams, or delegate further) and no MCP server connections. Best for single-shot consultations like 'ask researcher to look into X'."
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/delegate_task.rs:26`
