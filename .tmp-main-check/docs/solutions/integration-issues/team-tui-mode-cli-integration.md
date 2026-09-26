---
title: Team TUI Mode — CLI Integration with Channel-Based Worker
date: 2026-03-03
category: integration-issues
tags:
  - team-mode
  - tui
  - cli
  - channel-protocol
  - slash-command-gating
  - allowlist
modules:
  - crates/mika-cli/src/cli.rs
  - crates/mika-cli/src/main.rs
  - crates/mika-cli/src/commands/chat.rs
  - crates/mika-cli/src/tui/app.rs
  - crates/mika-cli/src/tui/commands/handlers.rs
  - crates/mika-cli/src/tui/ui.rs
  - crates/mika-cli/src/tui/input.rs
severity: medium
---

# Team TUI Mode — CLI Integration with Channel-Based Worker

## Problem

Teams only worked via `mika teams run <name> <goal>` (non-interactive CLI) or the
`/team` slash command which redirected users to leave the TUI. Users couldn't
interact with teams through the conversational TUI interface.

## Solution

Added `--team <name>` CLI flag (mutually exclusive with `--agent`, scoped to `chat` subcommand via `ChatArgs` — no longer global) that
launches the TUI in "team mode." User messages become goals sent to `run_team()`,
with progress streaming as system messages and deliverables displayed as assistant
responses via progressive reveal.

### Key Architectural Decisions

1. **Optional fields vs AppMode enum**: The `App` struct gained 4 optional team
   fields (`team_tx`, `team_rx`, `team_name`, `team_dir`) instead of refactoring
   into an `AppMode` enum. Rationale: `App` has 30+ fields accessed across many
   files — wrapping them in an enum would require touching 20+ call sites. The
   optional approach minimizes blast radius for an MVP. Tracked as future refactoring
   debt if a third mode is ever added.

2. **Allowlist over blocklist for slash commands**: Initially used a blocklist of
   agent-specific commands to block in team mode. Code review identified this as
   brittle — new commands would be silently allowed. Switched to an allowlist
   (`TEAM_MODE_ALLOWED_COMMANDS`) so new commands are blocked by default (safer
   failure mode).

3. **Settings loaded once at worker startup**: Initially loaded `Settings::load()`
   per-goal inside the team worker loop. Review identified this as unnecessary I/O.
   Moved to load once before the loop, consistent with the agent worker pattern.

4. **Dummy agent resources in team mode**: `App::new_team()` creates dummy
   `AsyncDatabase::in_memory()` and `ClaudeClient::dummy()` instances because the
   struct requires them. This is a known trade-off — the in-memory DB spawns an
   unused OS thread. Future refactoring to `Option<T>` fields or `AppMode` enum
   would eliminate this waste.

### Channel Protocol

```rust
enum TeamRequest {
    Goal(String),  // User message → team goal
    Quit,          // TUI exit
}

enum TeamResponse {
    Progress(String),    // "Decomposing goal...", "Running researcher..."
    Deliverable(String), // Final team output
    Error(String),       // Team execution failed
}
```

The progress callback captures an `mpsc::UnboundedSender<TeamResponse>` and sends
`Progress` messages through it. The TUI `tick_team_mode()` polls via `try_recv()`.

### Entry Point Flow

1. `main()` checks `cli.team` before `cli.agent` (early branch)
2. Validates team name format + existence
3. Gates subcommands (only `None`/`Chat` allowed)
4. `run_team()` spawns team worker, builds `App` in team mode, runs TUI event loop

## Gotchas

- **`reset_textarea()` placeholder**: After submitting a goal, the textarea
  placeholder reverted to "Type a message..." instead of "Type a goal for the
  team...". Fixed by checking `is_team_mode()` in `reset_textarea()`.

- **Export metadata**: Empty `session_id` and `model` in team mode produced
  broken export filenames (`session--timestamp.md`). Fixed with team-specific
  export labels.

- **Missing `validate_team_name()`**: The agent path validates name format before
  checking existence; the team path initially skipped this, producing a generic
  "not found" error for malformed names. Added parity.

- **Graceful shutdown**: Initial implementation used `abort()` directly on the
  team worker handle. Added a 2-second timeout window for graceful shutdown,
  consistent with the agent switch pattern.

## Testing

- CLI mutual exclusion: `--agent` and `--team` conflict (clap)
- `--team` parsing, `--agent` still works, `--team` with subcommand
- Allowlist verification: agent-specific commands blocked, universal commands allowed
- All existing 828 tests continue to pass
