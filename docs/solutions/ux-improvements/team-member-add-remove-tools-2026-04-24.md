---
title: "Add individual team member add/remove tools"
module: mika-agent/tools
tags: [team-management, tool-ergonomics, incremental-mutation]
problem_type: enhancement
date: 2026-04-24
---

# Add individual team member add/remove tools

## Problem

Modifying team composition required calling `update_team` with the full replacement `agents` array. For single-member add/remove operations, the caller had to recall the entire current roster, construct the complete new array, and submit it — verbose and error-prone for the common case of adding or removing one agent.

## Solution

Added two new management tools — `add_team_member` and `remove_team_member` — as thin wrappers around the existing team load-mutate-validate-persist pattern established by `update_team`.

### add_team_member

- Input: `team_name`, `agent` object (`name`, `role`, `mandate`)
- Validates: team exists, agent exists globally, agent not already in team, all fields non-empty and within `MAX_INPUT_LEN`
- Appends to `def.agents`, calls `validate_team`, serializes with `toml::to_string_pretty`, writes to `team.toml`

### remove_team_member

- Input: `team_name`, `agent_name`
- Validates: team exists, agent is a member, agent is not the orchestrator, removal won't drop below 2-member minimum
- Retains all agents except the target, calls `validate_team`, serializes and writes

### Key design decisions

1. **Orchestrator removal rejected.** `remove_team_member` returns an error when `agent_name == def.team.orchestrator`, directing the caller to use `update_team` to reassign the orchestrator first. This avoids a compound "remove + reappoint" operation.

2. **validate_team runs on both paths.** Both tools call `validate_team` after mutation, catching pre-existing orphans (agents listed in `team.toml` whose global definitions no longer exist). Orphans are surfaced with actionable error messages pointing to `update_team` — never silently tolerated.

3. **No shared helper extracted.** The load-normalize-validate-persist pattern is ~4 lines of TOML serialization + fs::write. Three tools share it (update, add, remove). Following rule-of-three, extraction would be premature — if a fourth team-mutating tool arrives, extract then.

## Files changed

- `crates/mika-agent/src/tools/add_team_member.rs` — new tool
- `crates/mika-agent/src/tools/remove_team_member.rs` — new tool
- `crates/mika-agent/src/tools/mod.rs` — module declarations and conditional registration
- `docs/architecture.md` — management tools count updated (10→12, 7→9 conditional), table rows added
- `crates/mika-agent/docs/architecture.md` — synced copy
- `crates/mika-agent/CLAUDE.md` — tool count and list updated

## Test coverage

13 inline tests across both tools:

| Tool | Test | Covers |
|------|------|--------|
| add_team_member | happy_path | Valid add, file updated, member count incremented |
| add_team_member | already_a_member | Duplicate detection, file unchanged |
| add_team_member | agent_not_exist | Global existence check |
| add_team_member | team_not_exist | Team existence check |
| add_team_member | invalid_team_name | Name validation |
| add_team_member | empty_fields | Empty team_name, name, role, mandate |
| add_team_member | preexisting_orphan | validate_team catches orphaned agent in roster |
| remove_team_member | happy_path | Valid removal from 3-member team |
| remove_team_member | not_a_member | Membership check |
| remove_team_member | is_orchestrator | Orchestrator guard, error references update_team |
| remove_team_member | would_drop_below_minimum | 2-member minimum enforcement |
| remove_team_member | team_not_exist | Team existence check |
| remove_team_member | preexisting_orphan | validate_team catches orphaned agent on removal |

## Patterns worth noting

- **Test fixture duplication.** `create_agent()` and `create_team_fs()` helpers are duplicated across `update_team.rs`, `add_team_member.rs`, and `remove_team_member.rs`. If a fourth tool needs them, extract to `test_utils`.
- **TOCTOU on team.toml.** Same non-atomic read-modify-write as `update_team`. Mitigated by the per-agent `tokio::Mutex` that serializes tool execution within a container.
- **Registration block.** Both tools registered inside the `agents.len() > 1 || !teams.is_empty()` conditional, alongside `update_team`. The orchestrator guard at the tool-registration level is inherited.
