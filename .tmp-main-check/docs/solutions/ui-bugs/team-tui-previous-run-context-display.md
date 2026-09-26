---
title: Show previous run context in team TUI when using --last-run or --run-id
category: ui-bugs
date: 2026-03-18
tags: [tui, team-engine, run-context, last-run, run-id]
modules: [mika-cli/tui/app, mika-cli/commands/chat]
---

# Show Previous Run Context in Team TUI

## Problem

When starting a team chat session with `--last-run` or `--run-id`, the TUI launched with an empty screen. The user had no visual indication they were continuing from a previous run — no context about what happened until they submitted a new goal.

## Root Cause

The `run_id` was passed through to the team worker for the engine's system prompt injection (via `build_previous_run_context()`), but the TUI's `App::new_team()` had no awareness of it. The `App` was always constructed with an empty `messages: Vec::new()`, regardless of whether a reference run existed.

## Solution

### 1. Pre-load summary in `run_team()` (chat.rs)

Load `TeamRunSummary` via `team_db.get_team_run_summary(ref_id)` after opening the DB but before constructing the App. Handle edge cases:

- **Not found:** Surface a warning system message to the TUI user
- **Team mismatch:** Log + surface a visible warning (not just `tracing::warn`)
- **DB error:** Log warning, continue without context

```rust
let (previous_run, run_warning) = if let Some(ref_id) = run_id {
    match team_db.get_team_run_summary(ref_id).await {
        Ok(Some(summary)) => {
            let warning = if summary.run.team_name != team_name {
                Some(format!("Warning: Referenced run belongs to team '{}', not '{}'",
                    summary.run.team_name, team_name))
            } else { None };
            (Some(summary), warning)
        }
        Ok(None) => (None, Some(format!("Warning: Referenced run {} not found.", ref_id))),
        Err(e) => { tracing::warn!(...); (None, None) }
    }
} else { (None, None) };
```

### 2. Extend `App::new_team()` signature (app.rs)

Added `previous_run: Option<TeamRunSummary>` parameter. When `Some`, format and inject as a `ChatMessage { role: ChatRole::System }` into the initial messages vec.

### 3. Format context block (app.rs)

`format_previous_run_context()` produces a styled block with box-drawing separators:

```
━━━ Previous Run Context ━━━
Run:    abcd1234 (completed)
Goal:   Build a REST API...
Time:   2026-03-17T14:30:00Z → 2026-03-17T14:45:00Z
Agents: alice, bob
Critic: Approved
Deliverable:
  REST endpoints created...
━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Field truncation uses character count (not byte length) for Unicode safety: goal at 200 chars, critic at 300, deliverable at 500.

## Key Decisions

- **Pre-load, don't query from App:** Keeps `App` synchronous and free of async DB concerns. The `run_team()` orchestration function is the right layer for data loading.
- **Reuse `TeamRunSummary`:** No intermediate DTO — the existing struct from `mika_agent::db` is imported directly. The coupling is data-only and matches existing patterns (App already imports from mika_agent).
- **`ChatRole::System` for rendering:** Matches existing team progress messages. Multi-line context works because system messages render line-by-line in red.
- **Char-count truncation over byte-length:** Avoids the UTF-8 byte-slicing panic pattern documented in `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`.

## Prevention

- When adding new TUI display features that need DB data, load it in the command function (`run_team`, `run_chat`) before constructing `App`, not inside the App itself.
- Always use character-count truncation (`s.chars().take(n).collect()`) for user-facing string truncation, never byte slicing.
- When logging warnings that affect UX, surface them visually in the TUI — `tracing::warn` is invisible to TUI users.

## Related

- Issue: #196
- `docs/solutions/architecture/team-conversation-continuity.md` — LLM-side previous run context injection
- `docs/solutions/architecture-patterns/team-run-last-run-shortcut-ux.md` — `--last-run` flag implementation
- `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md` — UTF-8 truncation lesson
