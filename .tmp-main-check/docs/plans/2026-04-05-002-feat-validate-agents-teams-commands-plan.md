---
title: "feat: add mika validate agents/teams commands"
type: feat
status: completed
date: 2026-04-05
---

# feat: add mika validate agents/teams commands

## Overview

Agent and team configs can be silently broken — wrong provider/model combinations, stale model fields from previous providers, max_tokens exceeding provider limits, missing API keys. These only surface at runtime. `mika skills validate` already catches skill config errors. This adds the same for agents and teams.

## Problem Statement

Switching providers via `/provider` leaves stale config fields that cause silent model switching and API errors (max_tokens out of range for DeepSeek, skill LLM override silently forcing OpenRouter). No pre-flight validation exists.

## Proposed Solution

Add `mika agents validate [NAME]` and `mika teams validate [NAME]` CLI subcommands following the existing `mika skills validate` pattern.

### Agent Checks

| # | Check | Level | Details |
|---|-------|-------|---------|
| 1 | Config loads | FAIL | `Settings::load_for_agent()` succeeds |
| 2 | Provider/model pairing | OK/WARN | Active provider's model field is set (WARN if using default) |
| 3 | Stale model fields | WARN | Model fields set for inactive providers |
| 4 | max_tokens range | FAIL/WARN | 0 = FAIL, >32768 = WARN |
| 5 | API key presence | FAIL | Active provider's API key set (skip Ollama) |
| 6 | soul.md exists | WARN | Agent has personality file |
| 7 | MCP config | FAIL/OK | Parse and validate mcp.json if present |
| 8 | Skill LLM overrides | WARN | Always-on skills overriding agent's provider |

### Team Checks

| # | Check | Level | Details |
|---|-------|-------|---------|
| 1 | team.toml loads | FAIL | TOML parses correctly |
| 2 | Name matches dir | WARN | `def.team.name` matches directory name |
| 3 | Orchestrator exists | FAIL | Agent exists on disk |
| 4 | Orchestrator in list | FAIL | Orchestrator listed in `[[agents]]` |
| 5 | All agents exist | FAIL | Each referenced agent exists |
| 6 | max_iterations range | WARN | 0 or >20 = WARN |

### Output Format

Same as `mika skills validate`:

```
agent-name/
    [OK]   config loaded — provider=anthropic, model=claude-sonnet-4-6
    [WARN] stale model field: deepseek_model="deepseek-chat" (active provider is anthropic)
    [OK]   API key present (MIKA_ANTHROPIC_API_KEY)
    [OK]   soul.md found

Summary: 3/3 valid, 0 with errors, 1 with warnings.
```

## Acceptance Criteria

- [x] `mika agents validate` validates all agents, prints diagnostics
- [x] `mika agents validate <name>` validates a single agent
- [x] `mika teams validate` validates all teams
- [x] `mika teams validate <name>` validates a single team
- [x] Output uses `[OK]`/`[WARN]`/`[FAIL]` badges (same as skills)
- [x] Non-zero exit on `[FAIL]` findings
- [x] All 8 agent checks implemented
- [x] All 6 team checks implemented
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## MVP

### Files to modify

1. **`crates/mika-cli/src/cli.rs`** — Add `Validate { name: Option<String> }` to `AgentsCommand` and `TeamsCommand` enums
2. **`crates/mika-agent/src/validate.rs`** — New module with `validate_agent()` and `validate_team_config()` returning `Vec<SkillDiagnostic>`
3. **`crates/mika-agent/src/lib.rs`** — Register `pub mod validate;`
4. **`crates/mika-cli/src/commands/agents.rs`** — Add `validate_agents()` handler matching `validate_skills()` pattern
5. **`crates/mika-cli/src/commands/teams.rs`** — Add `validate_teams()` handler

### Key reuse

- `SkillDiagnostic` / `DiagnosticLevel` from `mika_agent::skills::index` — reuse directly
- `Settings::load_for_agent()` from `mika_common::config` — load per-agent config
- `provider_fields()` / `ProviderKind::ALL` — iterate providers for stale field check
- `McpConfig::load()` + validation from `mika_agent::mcp::config`
- `scan_skills_dir()` from `mika_agent::skills::index` — skill LLM override check
- `team::load_team()` / `team::list_teams()` from `mika_common::team`

## Sources

- Related issue: #441
- Existing pattern: `crates/mika-cli/src/commands/skills.rs:954-1037` (validate_skills handler)
- Diagnostics types: `crates/mika-agent/src/skills/index.rs:392-434`
- Learning: `docs/solutions/architecture-patterns/config-key-registry-cli-management.md` — report all failures, never bail early
