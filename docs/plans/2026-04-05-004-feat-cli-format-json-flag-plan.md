---
title: "feat: add --format text|json flag to more CLI commands"
type: feat
status: completed
date: 2026-04-05
---

# feat: add --format text|json flag to more CLI commands

## Overview

Extend the existing `--format text|json` CLI flag pattern to 9 additional commands that currently only output human-readable text. This enables scripting, CI integration, and agent consumption of CLI output.

## Problem Statement

Several CLI commands only output human-readable text with headers, indentation, and colored markers. Three commands already support `--format text|json` (`ask`, `agents list`, `teams log`), but many others don't. Scripts that parse CLI output resort to fragile `tail | awk` patterns.

## Proposed Solution

Apply the documented pattern from `docs/solutions/architecture-patterns/cli-output-format-list-commands.md` to each command. The approach is mechanical: add the `format: OutputFormat` field to each subcommand's clap definition, then branch the handler on format to emit `serde_json::to_string_pretty` for JSON or the existing text output.

## Technical Approach

### Pattern (from solution doc)

1. Convert unit variant to struct variant in `Commands` enum (or add field to existing struct variant)
2. Add `#[arg(long, value_enum, default_value = "text")] format: OutputFormat`
3. Update dispatch to destructure and pass format
4. Branch handler: JSON emits structured data via `serde_json::json!()` or typed `Serialize` structs; text preserves existing output

### Types needing `Serialize`

Add `serde::Serialize` derive to these types in `mika-agent`:

| Type | File | Current Derives |
|------|------|----------------|
| `DiagnosticLevel` | `crates/mika-agent/src/skills/index.rs:394` | `Debug, Clone, Copy, PartialEq, Eq` |
| `SkillDiagnostic` | `crates/mika-agent/src/skills/index.rs:402` | `Debug, Clone` |
| `Person` | `crates/mika-agent/src/db.rs:213` | `Debug, Clone` |
| `Commitment` | `crates/mika-agent/src/db.rs:224` | `Debug, Clone` |
| `Preference` | `crates/mika-agent/src/db.rs:235` | `Debug, Clone` |
| `Event` | `crates/mika-agent/src/db.rs:265` | `Debug, Clone` |

Types already serializable (no changes needed): `TeamDefinition`, `TeamRunRow`, `SkillManifest`, `SkillInfo`, `LlmOverride`.

`ConfigKeyInfo` and `ConfigBackend` (`crates/mika-common/src/config.rs`) — use `serde_json::json!()` ad-hoc rather than deriving `Serialize` on these types, since `ConfigKeyInfo` has `&'static str` fields and a function pointer that can't be serialized.

### Commands to update

#### 1. `mika agents validate [NAME]` — `crates/mika-cli/src/commands/agents.rs:230`

- Add `format: OutputFormat` to `Validate` variant in `cli.rs:259`
- JSON schema: `[{"skill": "name", "level": "error|warning|info", "message": "..."}]`
- Add `Serialize` to `DiagnosticLevel` (with `#[serde(rename_all = "lowercase")]`) and `SkillDiagnostic`
- Non-zero exit still applies when errors present

#### 2. `mika teams validate [NAME]` — `crates/mika-cli/src/commands/teams.rs:270`

- Add `format: OutputFormat` to `Validate` variant in `cli.rs:309`
- Same JSON schema as agents validate (both use `Vec<SkillDiagnostic>`)

#### 3. `mika skills validate [NAME]` — `crates/mika-cli/src/commands/skills.rs:954`

- Add `format: OutputFormat` to `Validate` variant in `cli.rs:383`
- Same JSON schema as above

#### 4. `mika teams list` — `crates/mika-cli/src/commands/teams.rs:50`

- Convert `List` unit variant to struct variant with `format: OutputFormat` in `cli.rs:275`
- JSON schema: `[{"name": "team-name", "orchestrator": "agent", "agents": ["a", "b"], "flow": "sequential|parallel"}]`
- `TeamDefinition` already has `Serialize`

#### 5. `mika teams status <name>` — `crates/mika-cli/src/commands/teams.rs:136`

- Add `format: OutputFormat` to `Status` variant in `cli.rs:285`
- JSON schema: `{"team": {<TeamDefinition fields>}, "latest_run": {<TeamRunRow fields>} | null}`
- Both types already `Serialize`

#### 6. `mika skills list` — `crates/mika-cli/src/commands/skills.rs:59`

- Convert `List` unit variant to struct variant with `format: OutputFormat` in `cli.rs:327`
- JSON schema: `[{"name": "skill", "origin": "built-in|marketplace|custom", "enabled": true, "always_on": false, "tools": 3, "description": "...", "variants": 0, "llm_override": null | "provider/model"}]`
- Build with `serde_json::json!()` (ad-hoc, matching `agents list` pattern)

#### 7. `mika status` — `crates/mika-cli/src/commands/status.rs:1`

- Add `format: OutputFormat` to `StatusArgs` in `cli.rs:48`
- JSON schema: `{"agent": "name", "version": "0.x.y", "db_size_bytes": N, "messages": N, "people": N, "commitments": N, "preferences": N, "events": N, "tokens_used": N, "last_message": "ISO8601" | null}`
- Build with `serde_json::json!()` (all primitive values)

#### 8. `mika config list` — `crates/mika-cli/src/commands/config.rs:212`

- Add `format: OutputFormat` to `List` variant in `cli.rs:478`
- JSON schema: `[{"key": "llm_provider", "value": "anthropic" | null, "backend": "env|config|dotenv", "env_var": "MIKA_LLM_PROVIDER", "secret": false}]`
- Build with `serde_json::json!()` (avoid Serialize on `ConfigKeyInfo` due to function pointer)
- Secret values: redact in JSON same as text (show `***` or `null`)

#### 9. `mika memory search` — `crates/mika-cli/src/commands/memory.rs:28`

- Add `format: OutputFormat` to `Search` variant in `cli.rs:401`
- JSON schema: `{"people": [Person], "commitments": [Commitment], "preferences": [Preference], "events": [Event]}`
- Add `Serialize` to `Person`, `Commitment`, `Preference`, `Event`

## Acceptance Criteria

- [x] All 9 commands accept `--format text|json` (default `text`)
- [x] `--format text` output is identical to current behavior (no regressions)
- [x] JSON output goes to stdout; human status messages stay on stderr
- [x] Non-zero exit on errors still applies in JSON mode (validate commands)
- [x] Empty results: JSON emits `[]` or `null`, text preserves existing messages
- [x] Secret values redacted in JSON output (config list)
- [x] `Serialize` added to `DiagnosticLevel`, `SkillDiagnostic`, `Person`, `Commitment`, `Preference`, `Event`
- [x] `DiagnosticLevel` uses `#[serde(rename_all = "lowercase")]` for clean JSON keys
- [x] All tests pass (`cargo test`)
- [x] Clippy clean (`cargo clippy`)

## Sources & References

- Documented pattern: `docs/solutions/architecture-patterns/cli-output-format-list-commands.md`
- Flag scoping: `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`
- Existing implementations: `crates/mika-cli/src/commands/agents.rs:44` (agents list), `crates/mika-cli/src/commands/teams.rs:225` (teams log)
- Issue: #445, follow-up from #443
