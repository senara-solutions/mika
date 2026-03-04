---
status: pending
priority: p2
issue_id: "455"
tags: [code-review, agent-native, team-engine]
dependencies: []
---

# run_team Tool Discards Streaming Events in Server Mode

## Problem Statement

The `run_team` management tool passes `callback: None` in server mode, meaning Telegram users see nothing during a team run (up to 5 minutes). The TUI provides a rich live dashboard, creating a significant experience gap.

## Findings

- **Source**: Agent-native reviewer
- **Location**: `crates/mika-agent/src/tools/run_team.rs:85`
- **Evidence**: `callback: None` — no progress events reach the caller

## Proposed Solutions

### Option A: Wire MessageSender as callback (Recommended)
Pass a `TeamEventCallback` that formats key events (phase changes, agent completions) as text and sends via `MessageSender` so Telegram users get periodic updates.
- **Pros**: Closes biggest agent-native gap
- **Cons**: May produce chatty output
- **Effort**: Medium

### Option B: Persist events to DB, add polling tool
Persist `PhaseChanged` and `AgentStarted` to `team_messages`. Add a `get_team_progress` tool for live queries.
- **Pros**: Clean separation, reusable data
- **Cons**: Larger change, new tool needed
- **Effort**: Large

## Acceptance Criteria

- [ ] Server-mode users receive progress updates during team runs
- [ ] Updates include at minimum phase changes and agent completions
