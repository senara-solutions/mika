---
status: wont_fix
priority: p3
issue_id: 430
tags: [code-review, quality, duplication, tui]
dependencies: []
---

# ~50 lines of duplicated TUI loop boilerplate between run() and run_team()

## Problem Statement

`run()` (agent mode) and `run_team()` (team mode) in `crates/mika-cli/src/commands/chat.rs` share nearly identical boilerplate for panic hook, terminal setup, event loop dispatch, and terminal restore (~50 lines). This duplication means changes to the TUI setup must be applied in two places.

## Findings

- Source: pattern-recognition-specialist, code-simplicity-reviewer
- Identical code: panic hook (5 lines), terminal setup (10 lines), event reader (1 line), event dispatch loop (25 lines), terminal restore
- Agent mode has additional complexity (agent switching, reminder poller) that team mode doesn't need

## Proposed Solutions

### Option A: Extract shared `run_tui_loop()` helper
- Factor out panic hook + terminal setup + event loop + restore into a shared function
- Pass `&mut App` and a shutdown callback
- **Pros:** DRY, single source of truth for TUI lifecycle
- **Effort:** Medium (need to handle mode-specific shutdown)

### Option B: Leave as-is (tracked debt)
- Two modes may diverge further, making extraction harder later
- **Pros:** No blast radius now
- **Effort:** None

## Acceptance Criteria

- [ ] TUI lifecycle code exists in one place
- [ ] Both modes use the shared helper
- [ ] Mode-specific shutdown handled cleanly
