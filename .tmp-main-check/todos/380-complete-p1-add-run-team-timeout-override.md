---
status: complete
priority: p1
issue_id: 380
tags: [code-review, architecture, bug]
dependencies: []
---

# Add timeout_secs() override to RunTeamTool

## Problem Statement

`RunTeamTool` used the default 30-second tool timeout, but team workflows involve multiple agent iterations with Claude API calls that can take minutes. The tool would timeout prematurely.

## Findings

- **Source:** Architecture Strategist, Agent-Native Reviewer
- **File:** `crates/mika-agent/src/tools/run_team.rs`
- `delegate_task` correctly overrides to 120s but `run_team` did not

## Resolution

Added `fn timeout_secs(&self) -> Option<u64> { Some(300) }` to match the 5-minute agent loop total timeout.

## Work Log

- 2026-03-02: Added 300s timeout override to RunTeamTool
