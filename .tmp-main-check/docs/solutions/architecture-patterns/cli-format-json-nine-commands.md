---
title: "Extend --format text|json to 9 CLI commands"
module: mika-cli (cli.rs, commands/agents.rs, commands/teams.rs, commands/skills.rs, commands/status.rs, commands/config.rs, commands/memory.rs)
severity: low
tags:
  - cli
  - clap
  - json
  - output-format
  - scripting
  - serde
date: 2026-04-05
related_issues:
  - "#445"
  - "#443"
---

# Extend --format text|json to 9 CLI commands

## Problem

Only 3 CLI commands supported `--format text|json` (`ask`, `agents list`, `teams log`). The remaining commands output human-readable text only, making scripting and agent consumption difficult.

## Root Cause

The `OutputFormat` enum and pattern existed but hadn't been applied to other commands. Issue #445 (follow-up from #443) tracked the gap.

## Solution

Applied the documented pattern from `cli-output-format-list-commands.md` to 9 commands:

### Commands updated

1. **`agents validate`** — diagnostics as structured JSON
2. **`teams validate`** — diagnostics as structured JSON
3. **`skills validate`** — diagnostics as structured JSON
4. **`teams list`** — team definitions with agent counts
5. **`teams status`** — team definition + latest run
6. **`skills list`** — skill entries with origin, tools, badges
7. **`status`** — agent status with counts and db size
8. **`config list`** — config keys/values with backend info
9. **`memory search`** — structured people/commitments/preferences/events

### Types that needed `Serialize`

Added `serde::Serialize` derive to 6 types in `mika-agent`:

- `DiagnosticLevel` (with `#[serde(rename_all = "lowercase")]`)
- `SkillDiagnostic`
- `Person`, `Commitment`, `Preference`, `Event`

### Pattern applied

For each command:
1. Add `format: OutputFormat` field to clap subcommand variant
2. Update dispatch to pass format to handler
3. Branch handler: JSON uses `serde_json::json!()` + `to_string_pretty`; text preserves existing output
4. Empty results: JSON emits `[]` or `null`; text preserves helpful messages

### Key decision: ad-hoc JSON vs typed Serialize

`ConfigKeyInfo` has a function pointer field that can't be serialized, so `config list` uses `serde_json::json!()` ad-hoc values (same pattern as `agents list`). Memory types (`Person`, etc.) got `Serialize` derives since they're simple data structs with no unserializable fields.

## Prevention

When adding new CLI commands that produce structured output, include `--format text|json` from the start. Reference `cli-output-format-list-commands.md` for the pattern.
