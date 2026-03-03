---
status: pending
priority: p3
issue_id: "440"
tags: [code-review, agent-native, quality]
dependencies: []
---

# mika teams run CLI Subcommand Drops Most Event Types

## Problem Statement

In `commands/teams.rs`, the `mika teams run` CLI subcommand only prints `Progress` events. All other event types (`TasksAssigned`, `AgentCompleted`, `AgentFailed`, `CriticReview`) are silently dropped. This means `mika teams run <name> "<goal>"` shows significantly less detail than the TUI team mode, even without `/verbose`.

## Findings

- Agent-native reviewer flagged this asymmetry
- At minimum, agent completions, failures, and critic verdicts should be printed

## Proposed Solutions

### Option A: Print key events (Recommended)

Add match arms for `AgentCompleted`, `AgentFailed`, and `CriticReview` in the teams CLI callback, similar to the TUI callback pattern.

- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `mika teams run` prints agent completion/failure status
- [ ] Critic review verdict (approved/rejected) is printed
