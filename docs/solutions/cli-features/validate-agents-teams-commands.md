---
title: "Agent and team config validation commands"
category: cli-features
date: 2026-04-05
tags: [cli, validation, agents, teams, config, diagnostics]
issue: "#441"
---

# Agent and team config validation commands

## Problem

Agent and team configs can be silently broken — wrong provider/model combinations, stale model fields from previous providers, max_tokens exceeding provider limits, missing API keys. These only surface at runtime. `mika skills validate` existed but nothing equivalent for agents or teams.

## Root Cause

Switching providers via `/provider` leaves stale config fields (e.g., `deepseek_model` when active provider is `anthropic`). No pre-flight validation catches these mismatches before runtime API calls fail.

## Solution

Added `mika agents validate [NAME]` and `mika teams validate [NAME]` CLI subcommands following the `mika skills validate` pattern.

### Key design decisions

1. **Reuse `SkillDiagnostic`/`DiagnosticLevel`** from `skills/index.rs` — made `ok()`/`warn()`/`fail()` constructors `pub`. Same `[OK]`/`[WARN]`/`[FAIL]` output format.

2. **Validation logic in `crates/mika-agent/src/validate.rs`** — new module with `validate_agent()` and `validate_team_config()` returning `Vec<SkillDiagnostic>`.

3. **CLI handlers mirror `validate_skills()`** in `commands/agents.rs` and `commands/teams.rs` — iterate targets, collect diagnostics, print summary, `bail!` on errors.

### Agent checks (8 total)

Config loads, provider/model pairing, stale model fields, max_tokens range, API key presence (skip Ollama), soul.md exists, MCP config validation, skill LLM overrides.

### Team checks (6 total)

team.toml loads, name matches directory, orchestrator exists, orchestrator in agents list, all agents exist, max_iterations range.

### Files changed

- `crates/mika-agent/src/validate.rs` (new) — core validation logic
- `crates/mika-agent/src/lib.rs` — register module
- `crates/mika-agent/src/skills/index.rs` — `pub` constructors
- `crates/mika-cli/src/cli.rs` — `Validate` CLI variants
- `crates/mika-cli/src/commands/agents.rs` — handler
- `crates/mika-cli/src/commands/teams.rs` — handler

## Prevention

- Always run `mika agents validate` after switching providers via `/provider` or editing config.toml directly.
- Consider adding automatic validation to `mika agents switch` and startup flows in the future.
