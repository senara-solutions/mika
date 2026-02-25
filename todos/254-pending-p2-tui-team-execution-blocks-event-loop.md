---
status: pending
priority: p2
issue_id: 254
tags: [code-review, architecture, quality]
dependencies: []
---

# TUI Team Execution Blocks Event Loop

## Problem Statement

`handle_team()` in handlers.rs awaits `run_team()` directly in the TUI event loop. Team runs can take minutes (multiple LLM calls across planning, execution, and synthesis phases). The TUI freezes during execution, making it impossible for the user to scroll, cancel, or interact. The code itself has a comment acknowledging this: "this blocks the TUI".

## Findings

- **File:** `crates/mika-cli/src/tui/commands/handlers.rs` line 499
- The `handle_team()` function directly awaits the team run in the same task as the TUI event loop
- Team execution involves multiple sequential LLM calls (planning + N agent executions + synthesis), easily taking 1-3 minutes
- The existing chat flow already solves this pattern: `spawn_agent_worker()` in chat.rs uses `AgentRequest`/`AgentResponse` channels to run the agent on a background task
- No progress updates are shown to the user during team execution

## Proposed Solutions

Spawn the team run onto a separate task, following the `AgentRequest`/`AgentResponse` channel pattern already used by `spawn_agent_worker()` in chat.rs. Send progress callbacks through the channel so the TUI can display phase updates.

1. Create a `TeamRequest` / `TeamResponse` enum (or extend existing `AgentRequest`/`AgentResponse`)
2. Spawn team execution on a background tokio task
3. Send progress events (e.g., "Planning...", "Executing task 2/5...", "Synthesizing...") through the channel
4. TUI event loop receives and renders these updates without blocking

## Technical Details

- The `spawn_agent_worker()` pattern in chat.rs already demonstrates the correct architecture
- Progress callbacks could use the existing `mpsc` channel infrastructure
- Consider adding a cancellation mechanism (e.g., `CancellationToken`) so the user can abort long-running team executions
- The team engine would need to accept an optional progress callback or channel sender

## Acceptance Criteria

- [ ] TUI remains responsive during team execution (user can scroll, see updates)
- [ ] Progress updates are displayed in the chat area (e.g., current phase, agent progress)
- [ ] Team results are displayed when execution completes
- [ ] Error handling works correctly for background task failures
- [ ] Follows the same architectural pattern as `spawn_agent_worker()`

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Existing pattern: `crates/mika-cli/src/tui/chat.rs` (`spawn_agent_worker()`)
