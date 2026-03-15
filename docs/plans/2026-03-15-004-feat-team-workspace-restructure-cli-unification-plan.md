---
title: "feat: Team Workspace Restructure & CLI Unification"
type: feat
status: active
date: 2026-03-15
origin: docs/brainstorms/2026-03-15-team-workspace-restructure-brainstorm.md
---

# feat: Team Workspace Restructure & CLI Unification

## Overview

Three interconnected changes: (1) scope workspace directories per team run UUID, (2) unify CLI
so `mika ask --team` replaces `mika teams run`, (3) add `--run-id` for run continuity.

See brainstorm: [docs/brainstorms/2026-03-15-team-workspace-restructure-brainstorm.md](../brainstorms/2026-03-15-team-workspace-restructure-brainstorm.md)

## Problem Statement / Motivation

The current flat workspace (`~/.mika/teams/{team}/workspace/`) accumulates files across runs
with no isolation. PR #103 added conversation continuity via DB context injection, but the
filesystem artifacts are not scoped — agents in run 2 can see/overwrite run 1's files. Meanwhile,
the CLI has two separate paths for non-interactive team runs (`mika teams run` vs `mika ask`),
violating the orthogonality principle.

## Proposed Solution

### 1. Run-scoped workspace directories

```
~/.mika/teams/{team-name}/workspace/
  {run-uuid-1}/
    .meta/
      goal.md
      assignments.md
      critic_feedback.md
      deliverable.md
    analysis.txt          # agent output
    report.csv            # agent output
  {run-uuid-2}/
    .meta/
      ...
```

- Each `TeamEngine::new()` creates `workspace/{run_id}/` and `workspace/{run_id}/.meta/`
- Engine auto-writes metadata files into `.meta/` at each phase transition
- `.meta/` subdirectory prevents collisions with agent-chosen output filenames
- Agent workspace tools (`read_workspace`, `write_workspace`, `list_workspace`) operate on the
  run-scoped directory

### 2. `--run-id` for continuity

When `--run-id <uuid>` is provided:
- A NEW run is created (new UUID, new workspace directory) — it is NOT a resume
- The referenced run's workspace becomes read-only context:
  - `read_workspace` can read from both current and reference workspace (current takes precedence)
  - `list_workspace` shows files from both with `[current]`/`[ref:{short-uuid}]` labels
  - `write_workspace` only writes to the current run's workspace
- The orchestrator prompt includes the referenced run's context (goal, deliverable, agent results,
  critic feedback) via the existing PR #103 infrastructure, overriding the auto-detected
  "last completed run"
- Validation: run must exist, belong to the same team, not be in `running` status

For suspended runs: `--run-id` pointing to a suspended run still creates a new run (not resume).
The existing `execute_from_phase()` / task engine path handles true resume automatically.

### 3. CLI unification

**Add to `AskArgs`:**
- `--team <name>` (conflicts with `--agent`, `--task-id`, `--session`)
- `--run-id <uuid>` (requires `--team`)

**Add to `ChatArgs`:**
- `--run-id <uuid>` (requires `--team`)

**Remove:** `TeamsCommand::Run` variant. Keep `List`, `Create`, `Delete`, `Show`, `Status`, `Log`.

**Behavior of `mika ask --team`:**
- Runs full team cycle (decompose → execute → review → deliver)
- Prints progress to stderr, deliverable to stdout
- `--format json` extends schema: `{"role":"assistant","content":"...","team_run":{"run_id":"...","status":"...","iterations":N}}`
- Non-zero exit code on team run failure

## Technical Considerations

### Workspace tool plumbing

`team_tools()` signature changes from `team_tools(workspace_dir: &Path)` to
`team_tools(workspace_dir: &Path, reference_dir: Option<&Path>)`.

The three workspace tool structs gain an optional `reference_dir: Option<PathBuf>` field:
- `ReadWorkspaceTool`: checks `workspace_dir` first, falls back to `reference_dir`
- `WriteWorkspaceTool`: always writes to `workspace_dir` only
- `ListWorkspaceTool`: lists both, labels entries with provenance

### Engine init reordering

Currently `init_resources()` creates workspace_dir before `run_id` is generated. Reorder:
1. Generate `run_id` (UUID)
2. Compute `workspace_dir` = `team_workspace_base / run_id`
3. Create directory
4. Call `init_resources()` with the run-scoped path

### Metadata file writing

Add `write_metadata_file()` helper to `TeamEngine`:
```rust
fn write_metadata_file(&self, name: &str, content: &str) -> Result<()> {
    let path = self.workspace_dir.join(".meta").join(name);
    std::fs::write(&path, content)?;
    Ok(())
}
```

Called at:
- `decompose()` exit → write `goal.md` (once, at first decompose) and `assignments.md`
- `review()` exit → write `critic_feedback.md` (overwrite on each iteration)
- `deliver_phase()` exit → write `deliverable.md`

### `workspace_dir()` refactor

In `mika-common/src/team.rs`:
```rust
/// Base workspace directory for a team (contains per-run subdirectories)
pub fn workspace_base_dir(home_dir: &Path, team_name: &str) -> PathBuf {
    team_dir(home_dir, team_name).join("workspace")
}

/// Run-scoped workspace directory
pub fn workspace_run_dir(home_dir: &Path, team_name: &str, run_id: &str) -> PathBuf {
    workspace_base_dir(home_dir, team_name).join(run_id)
}
```

The old `workspace_dir()` is removed (callers migrate to `workspace_base_dir` or `workspace_run_dir`).

### CLI routing

In `main.rs`, extend the team-mode early-branch to handle `Some(Commands::Ask(_))`:
```rust
match (&team, &cli.command) {
    (Some(team_name), None | Some(Commands::Chat(_))) => { /* existing TUI path */ }
    (Some(team_name), Some(Commands::Ask(args))) => {
        commands::ask::run_team_ask(team_name, args, &settings, run_id).await?;
    }
    (Some(_), Some(other)) => bail!("--team is only supported with chat and ask"),
    (None, _) => { /* existing agent path */ }
}
```

### Server-side impact

The `run_team` management tool and `invoke_orchestrator` task dispatcher call the same
`teams::run_team()` function. Changes to `workspace_dir()` propagate automatically. The
`run_team` management tool does not need a `run_id` parameter — continuity via `--run-id` is
a CLI concern; the management tool auto-detects the previous run via DB queries (existing behavior).

Dashboard endpoint `/api/v1/team-runs/:id/summary` reads from `team_workspace` DB table —
unaffected. A future enhancement could add `/api/v1/team-runs/:id/workspace-files` to serve
the filesystem workspace.

## System-Wide Impact

- **Interaction graph**: `mika ask --team` → `run_team()` → `TeamEngine::execute()` → phase
  pipeline. Same call chain as `mika teams run` today, just routed from `ask` instead.
- **Error propagation**: Team run failures propagate as `anyhow::Error`. `mika ask` translates
  to non-zero exit code + stderr message. `--format json` includes error in response.
- **State lifecycle risks**: Run-scoped workspaces eliminate the stale-file accumulation problem.
  Metadata files in `.meta/` are engine-managed, not agent-writable (by convention — not enforced).
- **API surface parity**: The `run_team` management tool (used in server/agent mode) already
  provides equivalent functionality. No new API needed.

## Acceptance Criteria

### Functional Requirements

- [ ] `mika ask --team <name> "goal"` runs full team cycle and prints deliverable
- [ ] `mika ask --team <name> --format json "goal"` includes `team_run` metadata in JSON output
- [ ] `mika ask --team <name> --run-id <uuid> "goal"` creates new run with reference to previous
- [ ] `mika chat --team <name> --run-id <uuid>` works in interactive mode
- [ ] `--team` conflicts with `--agent`, `--task-id`, `--session` on `ask`
- [ ] `--run-id` requires `--team` on both `ask` and `chat`
- [ ] `--run-id` validation: exists, same team, not `running` status
- [ ] `mika teams run` is removed
- [ ] `mika teams list/create/delete/show/status/log` remain functional
- [ ] `mika teams create` output updated to reference `mika ask --team`
- [ ] Workspace created at `~/.mika/teams/{team}/workspace/{run-uuid}/`
- [ ] `.meta/goal.md` written after decompose phase
- [ ] `.meta/assignments.md` written after decompose phase (updated on re-decompose)
- [ ] `.meta/critic_feedback.md` written after review phase (overwritten each iteration)
- [ ] `.meta/deliverable.md` written after deliver phase
- [ ] `read_workspace` can read from both current and reference workspace
- [ ] `list_workspace` labels files with provenance when reference workspace exists
- [ ] `write_workspace` only writes to current run's workspace
- [ ] Team progress events printed to stderr in `ask --team` text mode
- [ ] Non-zero exit code on team run failure in `ask --team`

### Testing Requirements

- [ ] Test `--team` flag accepted on `ask` (invert existing `test_team_flag_rejected_on_ask`)
- [ ] Test `--team` conflicts with `--agent` on `ask`
- [ ] Test `--run-id` requires `--team`
- [ ] Test `--run-id` validation (invalid UUID, wrong team, running status)
- [ ] Test workspace tools with reference directory
- [ ] Test metadata file generation at each phase
- [ ] Test `--format json` extended schema for team runs
- [ ] Existing team engine tests pass with run-scoped workspaces

## Dependencies & Risks

- **Risk**: Workspace tools need two-path plumbing — adds complexity to simple tools. Mitigated
  by keeping `reference_dir` as `Option<PathBuf>` with no-op when `None`.
- **Risk**: Removing `mika teams run` is a breaking CLI change. Mitigated by being pre-1.0
  and documenting migration steps.
- **Risk**: Disk space from accumulated per-run workspace dirs. Mitigated by `mika teams delete`
  removing everything. No automatic cleanup needed now (YAGNI).

## Breaking Changes & Migration

**Before upgrading:**
1. Back up any files in `~/.mika/teams/*/workspace/` you want to keep

**After upgrading:**
1. Old workspace files in the flat `workspace/` directory are ignored (not migrated)
2. Replace `mika teams run <name> "goal"` with `mika ask --team <name> "goal"` in any scripts

## Implementation Phases

### Phase 1: Workspace restructure (engine + tools)

**Files:**
- `crates/mika-common/src/team.rs` — replace `workspace_dir()` with `workspace_base_dir()` + `workspace_run_dir()`
- `crates/mika-agent/src/teams/engine.rs` — reorder init to generate `run_id` before workspace dir; add `write_metadata_file()` helper; call at phase transitions
- `crates/mika-agent/src/tools/mod.rs` — update `team_tools()` signature to accept optional `reference_dir`
- `crates/mika-agent/src/tools/write_workspace.rs` — no change (writes to `workspace_dir` only)
- `crates/mika-agent/src/tools/read_workspace.rs` — add `reference_dir` fallback
- `crates/mika-agent/src/tools/list_workspace.rs` — add reference dir listing with provenance labels
- `crates/mika-agent/src/teams/prompt.rs` — update `workspace_listing()` to scan run-scoped dir

### Phase 2: CLI unification

**Files:**
- `crates/mika-cli/src/cli.rs` — add `--team` and `--run-id` to `AskArgs`, add `--run-id` to `ChatArgs`, remove `TeamsCommand::Run`
- `crates/mika-cli/src/main.rs` — extend team-mode early-branch to handle `Commands::Ask`, remove `teams run` dispatch, update `team_override()` for `Ask`
- `crates/mika-cli/src/commands/ask.rs` — add `run_team_ask()` function that calls `teams::run_team()` with progress callback to stderr, prints deliverable to stdout, handles `--format json` extended schema
- `crates/mika-cli/src/commands/teams.rs` — remove `run_team_cmd()`, update `create` output message
- `crates/mika-cli/src/commands/chat.rs` — pass `run_id` through to team worker

### Phase 3: `--run-id` plumbing

**Files:**
- `crates/mika-agent/src/teams/mod.rs` — add `reference_run_id: Option<&str>` parameter to `run_team()`
- `crates/mika-agent/src/teams/engine.rs` — in `TeamEngine::new()`, if `reference_run_id` is set: validate run, compute reference workspace path, pass to `init_resources()`; override `previous_run_summary` with specified run's summary
- `crates/mika-agent/src/db.rs` — add `get_team_run_by_id()` query (if not already present), add validation (team ownership, not running)
- `crates/mika-cli/src/commands/ask.rs` — pass `run_id` from args to `run_team()`
- `crates/mika-cli/src/commands/chat.rs` — pass `run_id` from args to team worker

### Phase 4: Cleanup and docs

**Files:**
- `crates/mika-cli/src/main.rs` — remove/invert `test_team_flag_rejected_on_ask` test
- `docs/` — update architecture docs for new workspace layout and CLI changes
- `CLAUDE.md` — update Directory Structure and Commands sections

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-15-team-workspace-restructure-brainstorm.md](../brainstorms/2026-03-15-team-workspace-restructure-brainstorm.md) — key decisions: run-scoped workspaces, `mika ask --team` replaces `mika teams run`, engine writes metadata files, `--run-id` for continuity
- Existing continuity implementation: PR #103 (`teams/prompt.rs:build_previous_run_context()`)
- CLI flag scoping pattern: `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`
- Team task child agent_id gotcha: `docs/solutions/database-issues/team-task-child-wrong-agent-id.md`
- DB consolidation rule: `docs/solutions/database-issues/consolidate-per-agent-team-dbs-into-single-container-db.md`
- Team engine code review findings: `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md`
