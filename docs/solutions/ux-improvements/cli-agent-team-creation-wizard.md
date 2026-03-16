---
title: "Guided CLI wizards for agent and team creation using dialoguer"
category: "ux-improvements"
date: "2026-03-16"
tags: ["cli", "dialoguer", "wizard", "interactive", "llm-generation", "tty-detection"]
components: ["mika-cli", "wizard", "agents", "teams"]
severity: "low"
---

# Guided CLI Wizards for Agent and Team Creation

## Problem

The Mika CLI's `mika agents create` and `mika teams create` commands had poor creation UX:

- **`agents create`** bootstrapped with generic defaults (`DEFAULT_SOUL`, `DEFAULT_IDENTITY`) and asked zero questions. Every new agent was identical until manually edited.
- **`teams create`** had interactive prompts but used raw `print!`/`stdin().read_line()` loops instead of the `dialoguer` crate already used consistently elsewhere in the codebase (`setup.rs`, `config.rs`, `skills.rs`).

Both commands needed a guided wizard that collects meaningful input, with the agent wizard optionally using the LLM provider to auto-generate a `soul.md` personality file.

## Root Cause

The original implementations were MVP scaffolding that was never upgraded:
- Agent creation was a single `home::bootstrap_agent()` call with no customization opportunity
- Team creation used manual `print!`/`read_line` loops instead of the project's standard `dialoguer` dependency, creating UX inconsistency

## Solution

Created a `wizard.rs` module in `crates/mika-cli/src/` with reusable interactive wizard functions built on the `dialoguer` crate, plus LLM-powered personality generation with graceful fallback.

### New Module: `wizard.rs`

**Agent Wizard (`run_agent_wizard`)**

Collects four inputs via `dialoguer` prompts, returning an `AgentWizardResult` struct:
1. **Display name** — `Input` with default derived from capitalizing the agent name
2. **Emoji** — `Input` with default `"✦"`
3. **Specialization** — `Input`, Enter to skip (empty string = use defaults)
4. **Communication style** — `Select` from presets (Professional and concise, Friendly and conversational, Technical and detailed, Custom...)

**Team Wizard (`run_team_wizard`)**

Takes the team name and list of existing agents, returning a `TeamWizardResult` struct:
1. **Orchestrator** — `Select` from agent list
2. **Members** — `MultiSelect` from remaining agents (orchestrator excluded)
3. **Per-member role and mandate** — `Input` with sensible defaults for each selected member
4. **Max iterations** — `Input` with default of 3
5. **Summary and confirmation** — `Confirm` before proceeding

**LLM Soul Generation (`generate_soul_md`)**

- Accepts an `&dyn LlmProvider`, agent name, specialization, and communication style
- Sends a structured prompt to generate a `soul.md` personality file
- Returns `Option<String>` — `None` on any error so the caller falls back to `template_soul_md()`

### Integration Pattern

```rust
// In agents.rs create():
let interactive = !no_interactive && std::io::stdin().is_terminal();
if interactive {
    let result = wizard::run_agent_wizard(&name)?;
    // Overwrite identity.toml with wizard answers
    // If specialization provided: try LLM generation -> fallback to template -> write soul.md
}
// Non-interactive: bootstrap_agent() only (existing behavior preserved)
```

### Design Patterns Used

- **TTY guard:** `std::io::stdin().is_terminal()` prevents wizard prompts in piped/CI contexts (same pattern as `setup.rs`)
- **`--no-interactive` CLI flag:** Added via clap to both `agents create` and `teams create`
- **Consistent `dialoguer` usage:** `Input`, `Select`, `MultiSelect`, `Confirm` — matching style in `setup.rs`
- **Graceful LLM degradation:** LLM provider call -> template fallback -> keep bootstrap defaults
- **Settings without agent:** `Settings::load(global_home)` reads global config + env vars, works before the agent directory exists

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-cli/src/wizard.rs` | **New** — all wizard logic, LLM generation, template fallback |
| `crates/mika-cli/src/cli.rs` | Added `--no-interactive` flag to both Create variants |
| `crates/mika-cli/src/commands/agents.rs` | Wired wizard, made `create()` async for LLM call |
| `crates/mika-cli/src/commands/teams.rs` | Replaced raw stdin with wizard, improved min-agents check (0 -> 2) |
| `crates/mika-cli/src/main.rs` | Added `mod wizard;` |

### Review Findings

Code review identified several follow-up items (tracked in `todos/681-687`):

1. **TOML injection (P1):** `identity.toml` written via `format!()` without escaping — should use `toml::to_string_pretty()` with a struct
2. **Missing timeout (P2):** LLM soul generation has no timeout — wrap with `tokio::time::timeout(30s)`
3. **Silent error swallowing (P2):** LLM errors discarded without logging — add `tracing::debug!`
4. **Dead flag (P2):** `--no-interactive` on teams create always bails — remove flag or add non-interactive support
5. **Duplicate binding (P2):** `agent_home` computed twice — hoist above if-block

## Prevention

### Config File Writes: Use Serializers, Never `format!()`

Any PR that writes a structured config file (TOML, JSON, YAML) must use a typed struct + serde serializer. The `format!()` approach is a TOML injection vector — user input containing quotes or newlines can corrupt the file or inject arbitrary sections.

### External API Calls: Enforce Timeouts

Every LLM or HTTP call outside the agent loop must have an explicit timeout. The agent loop has 30-second per-tool and 5-minute total timeouts; CLI wizard calls should use `tokio::time::timeout()` with similar bounds.

### Error Handling: No Silent Discards

Any `.ok()` or `Err(_) => None` on an external call must be accompanied by a `tracing::warn!` or `tracing::debug!`. Silent swallowing hides production issues.

### Checklist for New Interactive CLI Commands

- [ ] Uses `dialoguer` for all interactive input (no raw stdin)
- [ ] Two-phase design: collect inputs into a struct, then act on it
- [ ] `--no-interactive` flag works end-to-end with required args or sensible defaults
- [ ] Config file writes use serde serializers, never `format!()`
- [ ] External API calls have explicit timeouts
- [ ] Errors from external calls are logged, not silently discarded
- [ ] Every CLI flag has at least one test exercising its path
- [ ] Input validation happens inside dialoguer's `.validate_with()`
- [ ] Consistent UX with sibling commands

## Related Documentation

- [Setup Wizard Secret Handling](../security-issues/setup-wizard-secret-handling.md) — dialoguer patterns, TTY guards, TOML serialization
- [Multi-Provider LLM Trait Abstraction](../architecture-patterns/multi-provider-llm-trait-abstraction.md) — `LlmProvider` trait used for soul generation
- [CLI Flag Subcommand Scoping](../architecture-patterns/cli-flag-subcommand-scoping.md) — `AgentFlag` shared args pattern
- [Agent/Team Management Tools Integration](../integration-issues/agent-team-management-tools-integration.md) — `create_agent` tool (has same TOML injection pattern)
- [Config Key Rename Across Layers](../architecture-patterns/config-key-rename-across-layers.md) — checklist for config changes touching wizard/CLI
