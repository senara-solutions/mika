---
title: KG CLI management subcommands — mika kg status / list-agents / purge / validate
date: 2026-04-25
category: cli-features
module: mika-cli
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding CLI subcommands for inspecting or managing agent-scoped DB state
  - Building a destructive CLI operation that needs typed-ID confirmation
  - Extending an existing CLI with a new subcommand group
tags:
  - cli
  - knowledge-graph
  - typed-id-confirmation
  - purge
  - validate
  - subcommand-group
---

# KG CLI management subcommands — mika kg status / list-agents / purge / validate

## Context

The Knowledge Graph became per-agent-configurable with shared-corpus semantics in Milestone #17 (#778, #786, #787). Operators needed to inspect KG state, detect drift between configured and observed corpora, and clean up stale per-agent resolutions — all without raw SQL against `~/.mika/data/mika.db`. The `mika kg` subcommand group provides this operator surface, following the same pattern as `mika skills` and `mika agents`.

## Guidance

### Subcommand group pattern

Follow the single-file `commands/<name>.rs` pattern established by `commands/skills.rs`. One file, one `KgCommand` enum dispatched from `main.rs`. All subcommands accept `--format text|json` via the shared `OutputFormat` enum. `--agent` uses the `AgentFlag` pattern where applicable, with `agent_override()` on the args struct.

### Database helpers on `Database`

CLI commands use blocking `Database::open(&db_path)`. Since `conn` is `pub(crate)`, add purpose-specific helper methods to `Database` (e.g., `kg_count_rows`, `kg_observed_hashes`, `purge_kg_for_agent`, `kg_check_orphan_fk`). Each helper that takes table/column names uses an allowlist to prevent SQL injection — callers pass string identifiers, not raw SQL.

### Typed-ID confirmation for destructive operations

`mika kg purge` requires the operator to type the exact agent ID to confirm deletion. This is stricter than `y/n` confirmation — it prevents fat-finger deletions when multiple agents exist.

```
Type the agent ID to confirm: odds-engine-ceo
```

Key rules:
- **Strict string equality** — no normalization, no case folding, no trimming. `odds-engine-ceo` != `odds_engine_ceo`.
- **`is_terminal()` guard** — non-TTY contexts must pass `--yes` to bypass. Error message includes the exact command to copy-paste.
- **`--yes` flag** — for scripting. Named `--yes` (not `--force`) to match existing CLI conventions.
- **Pre-confirmation summary** — show exactly what will be deleted before prompting, so the operator can decide with full context.

### Three-way shared-corpus purge semantics

Default `purge --agent X` deletes only per-agent rows (`kg_subject_resolutions`, `kg_resolutions_log`). Shared-corpus rows (`kg_chunks`, `kg_subject_entities`, etc. keyed by `docs_root_hash`) are left intact because other agents may reference them.

`--include-orphaned-corpus` gates shared-layer deletion, but only when no other agent references the same `docs_root_hash`. The safety check runs in the CLI layer; the DB helper receives a `force_delete_shared: bool` pre-authorization — it trusts the caller.

### Validate as dedicated subcommand

`mika kg validate` follows the precedent of `mika agents validate` / `mika skills validate` — not folded into `mika doctor`. Each orphan FK check is a separate counted diagnostic with `[OK]`/`[WARN]`/`[FAIL]` output. Exit 0 when no Fail checks; exit 1 on any Fail. `[WARN]` (e.g., NULL `source_doc_hash` from pre-v26 rows) does not affect exit code.

### Char-safe string truncation

When truncating user-facing strings (like `docs_root` paths) for fixed-width table display, use `chars().count()` and `chars().rev().take(N)` instead of byte slicing. The `byte-slice-lint` CI job enforces this.

## Why This Matters

Without `mika kg`, operators resort to raw SQL for every KG inspection — a pattern that teaches the wrong mental model and is error-prone for cleanup operations. The typed-ID confirmation pattern is the first of its kind in the codebase and sets the precedent for future destructive CLI operations (e.g., `mika agents delete` could adopt it).

## When to Apply

- Adding a new CLI subcommand group for inspecting or managing agent-scoped state
- Implementing a destructive CLI operation where `y/n` confirmation is insufficient
- Building DB query helpers for CLI use when `conn` is not public
- Displaying KG or shared-corpus status to operators

## Examples

Typical operator workflow after a `docs_root` config change:

```bash
# 1. See which agents have drift
mika kg status

# 2. Purge stale resolutions for the drifted agent
mika kg purge --agent odds-engine-ceo

# 3. Verify no orphan rows remain
mika kg validate

# 4. JSON output for scripted monitoring
mika kg status --format json | jq '.agents[] | select(.drift == true)'
```

Non-interactive scripted cleanup:

```bash
mika kg purge --agent odds-engine-ceo --yes
```

## Related

- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — `AgentFlag` pattern
- `docs/solutions/architecture-patterns/cli-format-json-nine-commands.md` — `--format text|json` convention
- `docs/solutions/cli-features/validate-agents-teams-commands.md` — validate subcommand precedent
- `docs/solutions/ux-improvements/cli-agent-team-creation-wizard.md` — `dialoguer` + `is_terminal()` pattern
- GitHub issue: senara-solutions/mika#779
