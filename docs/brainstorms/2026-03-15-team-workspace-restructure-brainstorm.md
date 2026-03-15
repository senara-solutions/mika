# Team Workspace Restructure & CLI Unification

**Date:** 2026-03-15
**Status:** Decided

## What We're Building

Three interconnected changes to how teams work in Mika:

### 1. Run-scoped workspace directories

Change the workspace from a flat shared directory to per-run directories:

```
~/.mika/teams/{team-name}/workspace/
  {run-uuid-1}/
    goal.md              # auto-written by engine
    assignments.md       # auto-written by engine
    critic_feedback.md   # auto-written by engine (if review phase ran)
    deliverable.md       # auto-written by engine
    analysis.txt         # agent-written output file
    report.csv           # agent-written output file
  {run-uuid-2}/
    ...
```

Each team run creates its own subdirectory. The team engine auto-writes structured metadata files (goal, assignments, critic feedback, deliverable) at each phase. Agent output files also land here.

When `--run-id` references a previous run, the orchestrator gets:
- Previous run context via DB (PR #103, already built)
- Read access to the referenced run's workspace directory
- A new run directory for the current run's outputs

The orchestrator determines intent from the user's message + previous run state. No hardcoded interaction model.

### 2. CLI unification: `--team` on `ask` and `chat`

**Before:**
- `mika chat --team <name>` — interactive team mode
- `mika ask --agent <name> "message"` — one-shot agent
- `mika teams run <name> "goal"` — one-shot team (separate subcommand)

**After:**
- `mika chat --team <name>` — interactive team mode
- `mika ask --team <name> "goal"` — one-shot team (full cycle, prints deliverable)
- `mika ask --agent <name> "message"` — one-shot agent (unchanged)
- `--agent` and `--team` are mutually exclusive everywhere
- `--run-id <uuid>` available when `--team` is set (both ask and chat)

`mika teams run` is removed (redundant with `mika ask --team`). `mika teams list/create/delete/show` remain for team CRUD.

### 3. `--run-id` for run continuity

`--run-id <uuid>` tells the team engine to load the referenced run's context and workspace. Available on both `mika ask --team` and `mika chat --team`.

The new run always creates its own workspace directory. The previous run's workspace is read-only context.

## Why This Approach

- **Orthogonal CLI design:** One way to talk to agents (`--agent`), one way to talk to teams (`--team`), two modes (`chat` for interactive, `ask` for one-shot). The routing target and interaction mode are independent axes.
- **Run isolation:** Per-run directories prevent stale file accumulation and make it clear which artifacts belong to which run. The `team_workspace` DB table is already run-scoped — the filesystem should match.
- **Metadata on disk:** Auto-writing goal.md, assignments.md, etc. makes runs inspectable without DB queries. Users can browse `~/.mika/teams/{team}/workspace/` to see all past work.
- **Flexible continuity:** The orchestrator is an LLM — it figures out what to do with the previous run's context based on the user's message. No need to encode "refine" vs "continue" vs "new goal" in the CLI.

## Key Decisions

1. **Workspace scoped by run UUID** — each run gets `workspace/{run-uuid}/`
2. **Engine auto-writes metadata files** — goal.md, assignments.md, critic_feedback.md, deliverable.md written at each phase
3. **`mika ask --team`** runs full team cycle (decompose -> execute -> review -> deliver), prints deliverable
4. **Remove `mika teams run`** — replaced by `mika ask --team`
5. **Keep `mika teams list/create/delete/show`** — team CRUD stays
6. **`--run-id` on both ask and chat** — references previous run for continuity
7. **Not backward compatible** — provide manual migration steps

## Breaking Changes & Migration

**Before upgrading:**
1. Back up any files in `~/.mika/teams/*/workspace/` you want to keep
2. The flat workspace directory will no longer be used

**After upgrading:**
1. Old workspace files are not automatically migrated to run-scoped directories
2. `mika teams run` no longer exists — use `mika ask --team <name> "goal"` instead

## Open Questions

None — all decisions resolved during brainstorming.

## Implementation Scope (for /ce:plan)

### Workspace restructure
- Change `workspace_dir()` in `mika-common/src/team.rs` to accept a `run_id` parameter
- Update `TeamEngine` to create `workspace/{run_id}/` at run start
- Update `write_workspace`, `read_workspace`, `list_workspace` tools — they receive the run-scoped path
- Add metadata file writing to each engine phase (decompose, review, deliver)
- When `--run-id` is set, load previous run's workspace path as read-only context for orchestrator prompt

### CLI changes
- Add `--team` (conflicts with `--agent`) to `AskArgs`
- Add `--run-id` to both `AskArgs` and `ChatArgs` (requires `--team`)
- Implement team routing in `ask` command handler
- Remove `teams run` subcommand
- Update `--format json` to work with team deliverables

### Prompt changes
- Extend `build_orchestrator_context()` to include previous run's workspace file listing when `--run-id` is provided
- Metadata files become part of the workspace listing that the orchestrator already sees
