---
status: pending
priority: p1
issue_id: "729"
tags: [code-review, agent-native, observability]
dependencies: []
---

# Stale `event_type` enum in `query_timeline` tool definition

## Problem Statement

The `query_timeline` tool's JSON schema defines `"enum": ["message", "audit", "task"]` but the `unified_timeline` VIEW now emits `llm_call` and `tool_call` event types. The agent cannot filter timeline events by these new types because Claude respects JSON Schema enums strictly.

## Findings

- **Agent**: agent-native-reviewer
- **File**: `crates/mika-agent/src/tools/query_timeline.rs` lines 20-27
- **Evidence**: The enum list and description string are stale — missing `llm_call`, `tool_call`, `team_workspace`

## Proposed Solutions

### Option A: Update the enum (Recommended)
- Add `"llm_call"`, `"tool_call"`, `"team_workspace"` to the enum
- Update the description string
- **Effort**: Small (2-line change)
- **Risk**: None

## Acceptance Criteria

- [ ] `query_timeline(event_type="llm_call")` returns only LLM call events
- [ ] `query_timeline(event_type="tool_call")` returns only tool call events
- [ ] Tool description mentions all 6 event types

## Work Log

| Date | Action |
|------|--------|
| 2026-03-25 | Created from ce-review of PR #272 |
