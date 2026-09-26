---
title: "Show previous run context in TUI when using --last-run or --run-id"
type: feat
status: completed
date: 2026-03-18
---

# Show Previous Run Context in TUI When Using --last-run or --run-id

## Overview

When starting a team chat session with `--last-run` or `--run-id`, the TUI launches with an empty screen — no indication the session is continuing from a previous run. This feature injects a styled context block at the top of the chat area on startup, showing key information from the referenced run so the user knows where they're picking up from.

## Problem Statement

The `--last-run` and `--run-id` flags exist to continue from a previous team run, but the TUI provides zero visual feedback about the referenced run. The user has no context until they submit a new goal and the orchestrator's system prompt (invisibly) includes previous run data. This defeats the purpose of the continuation flags.

## Proposed Solution

On TUI startup when `run_id` is `Some`:

1. **Pre-load** the `TeamRunSummary` via `db.get_team_run_summary(run_id)` in `run_team()` before constructing the `App`
2. **Pass** the loaded `Option<TeamRunSummary>` into `App::new_team()`
3. **Inject** formatted context as system messages into `app.messages` during construction
4. **Render** using the existing `ChatRole::System` style (red text) with structured formatting

### Context Block Content

Display in a single system message with internal structure:

```
━━━ Previous Run Context ━━━
Run:    abc12345 (completed)
Goal:   Build a REST API for user management
Time:   2026-03-17T14:30:00Z → 2026-03-17T14:45:00Z

Agents: alice (completed), bob (completed)
Critic: Approved — all acceptance criteria met.

Deliverable:
  Created REST endpoints for CRUD operations...
━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### Field Rules

| Field | Source | Truncation | Missing behavior |
|---|---|---|---|
| Run ID | `run.id` | First 8 chars | Always present |
| Status | `run.status` | None | Always present |
| Goal | `run.goal` | 200 chars + "..." | Always present |
| Timestamps | `run.started_at`, `run.ended_at` | None | Show only `started_at` if `ended_at` is None |
| Agent results | `agent_results[].agent_name` + task status | Name + status only | Skip section if empty |
| Critic feedback | `critic_feedback` | 300 chars + "..." | Skip if None |
| Deliverable | `run.deliverable` | 500 chars + "..." | Show "(no deliverable)" if None |

## Technical Approach

### Phase 1: Data Loading & Plumbing

**Files:** `crates/mika-cli/src/commands/chat.rs`, `crates/mika-cli/src/tui/app.rs`

1. In `run_team()` (chat.rs:647), after opening the `team_db` (line 656) and before constructing the `App` (line 712):
   - If `run_id` is `Some`, call `team_db.get_team_run_summary(run_id)` to load the summary
   - Validate team name matches: `summary.run.team_name == team_name` (warn if mismatched)
   - Handle errors gracefully: log warning, continue with `None`

2. Extend `App::new_team()` signature to accept `previous_run: Option<TeamRunSummary>`

3. In `App::new_team()`, if `previous_run` is `Some`, format and inject the context as a `ChatMessage { role: ChatRole::System, ... }` into `self.messages`

### Phase 2: Context Formatting

**Files:** `crates/mika-cli/src/tui/app.rs`

Add a private helper function `format_previous_run_context(summary: &TeamRunSummary) -> String` that:

- Builds the structured text block shown above
- Truncates long fields (goal: 200 chars, critic: 300 chars, deliverable: 500 chars)
- Handles missing optional fields gracefully
- Includes abbreviated run ID (first 8 chars)
- Lists agent names with their completion status
- Uses box-drawing characters for visual separation (━━━)

### Phase 3: Edge Cases

**Files:** `crates/mika-cli/src/commands/chat.rs`

Handle these cases in `run_team()`:

| Case | Behavior |
|---|---|
| `get_team_run_summary()` returns `Ok(None)` | Inject warning system message: "Referenced run {run_id} not found" |
| `get_team_run_summary()` returns `Err(e)` | Log warning, continue without context (no message) |
| Team name mismatch | Inject warning: "Referenced run belongs to team '{other}', not '{current}'" |
| Run status is "running" | Show context with note: "(still in progress — context may be incomplete)" |
| Run status is "suspended" | Show context with note: "(suspended — has pending callbacks)" |

## Acceptance Criteria

- [x] `mika --team myteam --last-run` shows context block from the most recent finished run
- [x] `mika --team myteam --run-id <uuid>` shows context block from the specified run
- [x] `mika --team myteam` (no flags) shows no context block (existing behavior unchanged)
- [x] Context block includes: run ID, status, goal, timestamps, agent results, critic feedback, deliverable
- [x] Long fields are truncated with "..." indicator
- [x] Missing optional fields (deliverable, critic, agents) are handled gracefully
- [x] Invalid/non-existent run ID shows a warning message instead of silent failure
- [x] Team name mismatch shows a warning
- [x] Context block renders as styled system message(s) in the chat area

## Key Files

| File | Change |
|---|---|
| `crates/mika-cli/src/commands/chat.rs:647` | Load summary in `run_team()`, pass to `App::new_team()` |
| `crates/mika-cli/src/tui/app.rs:549` | Extend `new_team()` signature, inject context messages |
| `crates/mika-agent/src/db.rs:348` | No changes — `TeamRunSummary` and queries already exist |
| `crates/mika-agent/src/async_db.rs:1257` | No changes — `get_team_run_summary()` already exists |

## Dependencies & Risks

- **No new dependencies** — all data structures and queries already exist
- **Low risk** — purely additive UI change, no database modifications, no new API calls
- **`TeamRunSummary` is already `pub`** — can be imported by mika-cli without changes

## Sources

- Related issue: #196
- Existing DB queries: `crates/mika-agent/src/db.rs` — `get_team_run_summary()`, `TeamRunSummary`, `TeamRunRow`
- Team TUI architecture: `docs/solutions/integration-issues/team-tui-mode-cli-integration.md`
- Previous run continuity: `docs/solutions/architecture/team-conversation-continuity.md`
- `--last-run` implementation: `docs/solutions/architecture-patterns/team-run-last-run-shortcut-ux.md`
