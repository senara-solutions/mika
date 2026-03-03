---
title: "Adding a prompt-only built-in skill to the bundled skills system"
date: "2026-03-03"
module: "skills"
severity: "low"
tags:
  - "built-in-skills"
  - "prompt-only"
  - "bundled-skills"
  - "keyword-triggered"
related_files:
  - "crates/mika-agent/src/bundled_skills.rs"
  - "crates/mika-agent/templates/skills/agents-teams/skill.toml"
  - "crates/mika-agent/templates/skills/agents-teams/system_prompt.md"
  - "crates/mika-agent/templates/skills/agents-teams/tools.json"
  - "crates/mika-agent/templates/skills/mcp/skill.toml"
---

# Adding a Prompt-Only Built-In Skill

## Problem Statement

When adding a new prompt-only built-in skill (one that provides system prompt guidance but no tools of its own), the developer needs to follow a specific pattern involving 3 template files and 1 Rust registration. The pattern is strict — compile-time `include_str!` validation means missing files fail the build, and overly broad keywords cause false positives that waste context tokens.

## Root Cause

Not a bug — this documents the procedure and key design decisions for adding prompt-only bundled skills, using the `agents-teams` skill (added 2026-03-03) as the reference implementation.

## Solution

### Files Required

A prompt-only bundled skill needs exactly 3 template files + 1 Rust change:

```
crates/mika-agent/templates/skills/{skill-name}/
├── skill.toml          # Manifest with metadata and keywords
├── system_prompt.md    # Behavioral guidance injected into system prompt
└── tools.json          # Empty array [] (required for include_str!)
```

### Step 1: skill.toml

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

Key fields:
- `always_on = false` for most prompt-only skills (avoids token waste)
- `timeout_secs = 10` matches other prompt-only skills (`mcp`, `self-knowledge`)
- `version = "0.1.0"` consistent with all bundled skills

### Step 2: tools.json

```json
[]
```

This file **must exist** even though it's empty. The `skill!` macro uses `include_str!()` at compile time, which requires the file to be present or the build fails.

### Step 3: system_prompt.md

Write behavioral guidance that **complements** (not duplicates) the existing base system prompt sections in `prompt.rs`. Focus on:
- When to use which tool (decision trees)
- Tool-specific limitations and timeouts
- Fallback guidance for when tools aren't available

### Step 4: Register in bundled_skills.rs

```rust
static AGENTS_TEAMS_SKILL: BundledSkill = skill!("agents-teams", [
    ("skill.toml" => "../templates/skills/agents-teams/skill.toml"),
    ("system_prompt.md" => "../templates/skills/agents-teams/system_prompt.md"),
    ("tools.json" => "../templates/skills/agents-teams/tools.json"),
]);
```

Then add `&AGENTS_TEAMS_SKILL` to the `BUNDLED_SKILLS` array.

Naming: `SCREAMING_SNAKE_CASE` for the static (kebab-case → snake_case: `agents-teams` → `AGENTS_TEAMS_SKILL`).

## Key Design Decisions

### Keyword Selection

**Avoid overly broad keywords.** The matcher uses `message_lower.contains(kw)` substring matching.

| Keyword | Risk | Decision |
|---------|------|----------|
| "agent" | Matches "insurance agent", "reagent" | Removed — too many false positives |
| "team" | Matches "steam", "team meeting" | Removed — common in assistant context |
| "delegate" | Specific enough in practice | Kept |
| "delegate task" | Very specific | Kept |
| "run team" | Very specific | Kept |

### always_on Choice

Use `always_on = false` when the skill's guidance is only relevant for specific user intents. The `agents-teams` skill is false because management tools are conditional — they only exist when `agents.len() > 1 || !teams.is_empty()`.

### System Prompt Layering

The base `prompt.rs` already includes a dynamic "Agents & Teams" section listing available agents and teams. The skill's `system_prompt.md` adds behavioral guidance (when to use `delegate_task` vs `run_team`, delegate limitations, timeouts). These are complementary, not redundant.

### Handling Single-Agent Setups

The skill is bundled in every agent's `skills/` directory unconditionally, but management tools are only registered when multiple agents or teams exist. The `system_prompt.md` includes a fallback note: "If these management tools are not listed in your available tools, the user has not configured multiple agents or teams yet."

## Prevention Checklist

When adding a new prompt-only bundled skill:

- [ ] Create `templates/skills/{name}/skill.toml` with `[skill]` and `[triggers]` sections
- [ ] Create `templates/skills/{name}/tools.json` with `[]`
- [ ] Create `templates/skills/{name}/system_prompt.md` with behavioral guidance
- [ ] Add `skill!` static in `bundled_skills.rs` (no `+x` suffix — prompt-only has no executables)
- [ ] Add to `BUNDLED_SKILLS` array
- [ ] Run `cargo test -p mika-agent` (`test_seed_creates_all_skills` auto-covers)
- [ ] Run `cargo clippy`
- [ ] Update `docs/skills.md` prompt-only skills table
- [ ] Verify keywords don't overlap with existing skills
- [ ] Verify `system_prompt.md` doesn't duplicate `prompt.rs` content

## Common Mistakes

1. **Forgetting `tools.json`** — Even empty, the file must exist for `include_str!`
2. **Bare common-word keywords** — "agent", "team", "help" cause frequent false positives
3. **Duplicating base prompt content** — Read `prompt.rs` first to avoid redundancy
4. **Not testing** — `cargo test` catches missing files and invalid TOML/JSON at compile time

## Related Documentation

- [ADR-002: Filesystem-Based Skill Registry](../../adr/002-filesystem-skill-registry.md)
- [ADR-004: Multi-Agent Teams Orchestration](../../adr/004-multi-agent-teams-orchestration.md)
- [ADR-006: Git-Based Skills Marketplace](../../adr/006-git-based-skills-marketplace.md)
- [Agent and Team Management Tools Integration](agent-team-management-tools-integration.md)
- [Skills System Documentation](../../skills.md)
