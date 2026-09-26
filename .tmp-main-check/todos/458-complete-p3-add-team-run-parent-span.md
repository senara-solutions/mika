---
status: complete
priority: p3
issue_id: "458"
tags: [code-review, telemetry, team-engine]
dependencies: []
---

# Add team_run Parent Span for Trace Hierarchy

## Problem Statement

The team engine logs `info!("team_run started")` but does not wrap the entire execution in an `info_span!("team_run", ...)`. Without a parent span, per-agent `team_agent_task` spans appear as siblings rather than children, making trace visualization in Langfuse/Jaeger flat rather than hierarchical.

## Proposed Fix

Wrap `TeamEngine::execute()` in `info_span!("team_run", team = %self.team_name, goal_len = goal.len())` and `.instrument()` the async execution.

- **Location**: `crates/mika-agent/src/teams/engine.rs`
- **Effort**: Small

## Acceptance Criteria

- [ ] `team_run` span is parent of `team_agent_task` spans in trace viewer
