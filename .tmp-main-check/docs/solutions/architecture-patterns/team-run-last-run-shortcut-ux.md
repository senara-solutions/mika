---
title: "Add --last-run shortcut and enhance teams log for run-id discovery"
category: architecture-patterns
date: 2026-03-18
tags:
  - ux
  - team-runs
  - cli-flags
  - database-query
  - testing
affected_modules:
  - crates/mika-cli/src/cli.rs
  - crates/mika-cli/src/main.rs
  - crates/mika-cli/src/commands/teams.rs
  - crates/mika-agent/src/db.rs
related:
  - docs/solutions/architecture-patterns/cli-flag-id-suffix-convention.md
  - docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md
  - docs/solutions/architecture/team-conversation-continuity.md
  - docs/solutions/security-issues/team-workspace-ref-dir-validation-hardening.md
  - docs/solutions/database-issues/team-graph-persistence-replacing-toml-history.md
---

# Add `--last-run` Shortcut and Enhance `teams log` for Run-ID Discovery

## Problem

Continuing from a previous team run required finding the full 36-character UUID to pass via `--run-id <uuid>`. This was friction-heavy:

1. `mika teams log` truncated run IDs to 8 characters, making them unusable for copy-paste
2. No shortcut existed for the most common case: "continue from the last finished run"
3. The hardcoded limit of 50 runs and text-only output provided no machine-readable alternative

Users had to resort to direct database queries or other workarounds to locate the full UUID.

## Root Cause

The `teams log` text output used `run.id.get(..8).unwrap_or(&run.id)` to truncate UUIDs for visual compactness. There was no convenience flag to bypass the UUID lookup entirely.

## Solution

### 1. `--last-run` Flag (cli.rs)

Added a boolean flag to both `ChatArgs` and `AskArgs` with clap's declarative constraint system:

```rust
/// Continue from a previous team run (requires --team)
#[arg(long, requires = "team", conflicts_with = "last_run")]
pub run_id: Option<String>,

/// Use the most recent finished team run as context (requires --team)
#[arg(long, requires = "team", conflicts_with = "run_id")]
pub last_run: bool,
```

Bidirectional conflict ensures clap rejects `--last-run --run-id` at parse time.

### 2. Database Query (db.rs)

New `get_last_finished_team_run(team_name)` method returns the most recent run in a terminal state:

```sql
SELECT {TEAM_RUN_COLUMNS} FROM team_runs r
JOIN teams t ON r.team_id = t.id
WHERE t.name = ?1 COLLATE NOCASE
  AND r.status NOT IN ('running', 'suspended')
ORDER BY r.started_at DESC LIMIT 1
```

Uses `NOT IN` (exclusion) rather than `IN` (inclusion) so new terminal statuses are automatically included. Returns completed, failed, and cancelled runs; excludes running and suspended (which are resumable, not finished).

**Semantic distinction from `get_last_completed_team_run`:** The existing method uses `IN ('completed', 'failed', 'suspended')` and is used for team conversation continuity (where suspended context is valuable). The new method excludes suspended runs because `--last-run` targets truly finished work.

### 3. Resolution Helper (main.rs)

```rust
fn resolve_last_run(global_home: &Path, team_name: &str) -> anyhow::Result<String> {
    let db = commands::teams::open_container_db(global_home)?;
    match db.get_last_finished_team_run(team_name)? {
        Some(run) => Ok(run.id),
        None => anyhow::bail!(
            "No finished team run found for team '{team_name}'. \
             Run the team first before using --last-run."
        ),
    }
}
```

Wired into both Chat and Ask branches. UUID validation still runs after resolution.

### 4. Enhanced `teams log` (commands/teams.rs)

- Full UUIDs in text output (was truncated to 8 chars)
- `--format json` outputs array of objects: `id`, `team_name`, `goal`, `status`, `started_at`, `ended_at`
- `-n/--limit` defaults to 10 (was hardcoded 50)
- `open_container_db` changed to `pub(crate)` for reuse from `main.rs`

## Prevention Strategies

### 1. Always Provide Convenience Shortcuts Alongside Explicit Identifiers

Any `--id <uuid>` flag should have a corresponding shortcut for the common case. Before adding an ID flag, ask: "What is the most common way a user will obtain this ID?" If the answer is "run another command and copy it," add a shortcut flag.

### 2. Never Truncate Identifiers That Users Need to Copy-Paste

Text output must show the full value for anything another command accepts as input. Truncate descriptions or timestamps before truncating IDs.

### 3. Test Every Clap Constraint Annotation

Each `requires`/`conflicts_with` must have a corresponding test. Clap constraints are declarative and silent -- misconfiguration produces no compiler errors. The code review caught 5 missing tests.

**Checklist for new flag pairs:**
- Flag A alone works
- Flag B alone works
- Both together rejected (if `conflicts_with`)
- Dependency enforced (if `requires`)

### 4. Document Semantic Differences Between Similar DB Methods

When adding a query method close to an existing one, the doc comment must state how it differs and when to use each. The `NOT IN` vs `IN` choice should match intent: exclusion for "anything finished" (future-proof), inclusion for "only these specific states" (explicit).

## Pre-Implementation Checklist

Before merging any new CLI subcommand or flag:

1. Does any flag accept an opaque identifier? Add a convenience shortcut for the common case.
2. Does text output show full values for anything another command accepts as input?
3. Are all clap constraint annotations covered by tests?
4. Do new DB methods have doc comments explaining semantic boundaries vs similar methods?
5. Check for existing reusable patterns: `OutputFormat`, `open_container_db`, `format_ts`, `TEAM_RUN_COLUMNS`.
