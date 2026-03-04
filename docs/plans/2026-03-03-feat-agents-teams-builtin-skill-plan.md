---
title: "feat: Add agents-teams built-in skill"
type: feat
status: completed
date: 2026-03-03
---

# Add `agents-teams` Built-in Skill

## Overview

Add a prompt-only built-in skill that provides behavioral guidance for delegating tasks to agents and running team workflows. Follows the exact pattern of the existing `mcp` skill (keyword-triggered, no tools, `tools.json` = `[]`).

## Acceptance Criteria

- [x] `crates/mika-agent/templates/skills/agents-teams/skill.toml` — manifest with keywords and metadata
- [x] `crates/mika-agent/templates/skills/agents-teams/tools.json` — empty array `[]` (required for `include_str!` compilation)
- [x] `crates/mika-agent/templates/skills/agents-teams/system_prompt.md` — behavioral guidance for the 6 management tools
- [x] Skill registered in `crates/mika-agent/src/bundled_skills.rs` via `skill!` macro + `BUNDLED_SKILLS` array
- [x] `cargo test` passes (existing `test_seed_creates_all_skills` auto-covers new skill)
- [x] `cargo clippy` clean

## MVP

### 1. `crates/mika-agent/templates/skills/agents-teams/skill.toml`

```toml
[skill]
name = "agents-teams"
description = "Guidance for delegating tasks to agents and running team workflows"
version = "0.1.0"
always_on = false
timeout_secs = 10

[triggers]
keywords = ["delegate", "delegate task", "run team", "list agents", "list teams", "team workflow", "team status", "team history", "multi-agent"]
```

**Design decisions:**
- `always_on = false` — management tools are conditional (only present with >1 agent or teams); injecting guidance on every message wastes tokens for single-agent users
- `timeout_secs = 10` — matches `mcp` skill (prompt-only, no tool execution)
- Keywords tightened from original spec: removed bare "agent", "agents", "team", "teams" to avoid false positives (these words appear constantly in executive assistant conversations: "my team meeting", "insurance agent", "steam"). Kept "delegate" as it is specific enough. Added tool-name phrases and "multi-agent" for intentional queries.

### 2. `crates/mika-agent/templates/skills/agents-teams/tools.json`

```json
[]
```

### 3. `crates/mika-agent/templates/skills/agents-teams/system_prompt.md`

Content should cover (without repeating the dynamic "Agents & Teams" section already in the base system prompt):

**When to use each tool:**
- `list_agents` — discover available agents and their roles before delegating
- `delegate_task` — single-shot consultation ("ask researcher to look into X"). 120s timeout.
- `list_teams` — discover configured teams and their composition
- `run_team` — multi-agent orchestrated workflow for complex goals. 300s timeout.
- `get_team_status` — check progress of a team's most recent (or specific) run
- `get_team_history` — list recent runs for a team (default 5, max 20)

**Decision guidance:**
- Use `delegate_task` for quick, single-agent consultations
- Use `run_team` for goals that benefit from decomposition across multiple specialists
- Always call `list_agents` or `list_teams` first if unsure what is available

**Delegate limitations:**
- Delegates have their own personality, memory, and skills
- Delegates CANNOT: delegate further, run teams, connect to MCP servers, access your memory/conversation
- Write clear, self-contained task descriptions (delegates have no context from your conversation)

**Timeouts:**
- `delegate_task`: 120s — for tasks exceeding this, break into smaller sub-tasks
- `run_team`: 300s — for full multi-agent orchestration

**Fallback note:**
- If management tools are not in the available tool list, the user has not configured multiple agents or teams yet

### 4. `crates/mika-agent/src/bundled_skills.rs`

Add static declaration and array entry:

```rust
static AGENTS_TEAMS_SKILL: BundledSkill = skill!("agents-teams", [
    ("skill.toml" => "../templates/skills/agents-teams/skill.toml"),
    ("system_prompt.md" => "../templates/skills/agents-teams/system_prompt.md"),
    ("tools.json" => "../templates/skills/agents-teams/tools.json"),
]);
```

Add `&AGENTS_TEAMS_SKILL` to `BUNDLED_SKILLS` array (9th entry).

## Context

### Key files
- `crates/mika-agent/src/bundled_skills.rs` — registration site
- `crates/mika-agent/templates/skills/mcp/` — closest analog (prompt-only skill)
- `crates/mika-agent/src/tools/delegate_task.rs` — delegate_task implementation
- `crates/mika-agent/src/tools/run_team.rs` — run_team implementation
- `crates/mika-agent/src/prompt.rs:191-234` — existing "Agents & Teams" system prompt section

### SpecFlow findings incorporated
- Tightened keywords to avoid false positives (removed bare "agent"/"team")
- Added `tools.json` with `[]` (required for `include_str!` at compile time)
- System prompt complements (not duplicates) the existing base prompt section
- Included fallback note for single-agent setups where tools are absent

## References

- Existing `mcp` skill: `crates/mika-agent/templates/skills/mcp/`
- ADR-004: Multi-Agent Teams Orchestration
- ADR-002: Filesystem Skill Registry
