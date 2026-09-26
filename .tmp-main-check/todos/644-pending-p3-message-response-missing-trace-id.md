---
status: pending
priority: p3
issue_id: 644
tags: [code-review, dashboard, api]
dependencies: []
---

# Add trace_id to MessageResponse and TraceMessage TypeScript type

## Problem Statement

The `MessageResponse` struct in `dashboard.rs` and the `TraceMessage` TypeScript interface in `timeline.ts` do not include a `trace_id` field. The DB query now correctly fetches `m.trace_id` (fixed in this PR), but it is dropped during `From<SessionMessage>` conversion. Not a functional issue today (the trace_id is in the URL path), but keeps API types incomplete.

## Findings

- `MessageResponse` struct at `crates/mika-agent/src/server/dashboard.rs:324-333` has no `trace_id`
- `From<SessionMessage>` impl at line 335 silently drops `trace_id`
- `TraceMessage` interface at `dashboard/src/api/timeline.ts:51-60` also lacks `trace_id`
- Flagged by: architecture-strategist, agent-native-reviewer, security-sentinel, pattern-recognition-specialist

## Proposed Solutions

### Option A: Add trace_id to both (Recommended)
Add `trace_id: Option<String>` to `MessageResponse` and `trace_id?: string` to `TraceMessage`.
- Pros: API contract matches DB schema, enables future per-message trace linking
- Cons: Slightly larger JSON payloads
- Effort: Small
- Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/server/dashboard.rs`, `dashboard/src/api/timeline.ts`

## Acceptance Criteria

- [ ] `MessageResponse` includes `trace_id: Option<String>`
- [ ] `From<SessionMessage>` maps `trace_id`
- [ ] `TraceMessage` TypeScript interface includes `trace_id?: string`
- [ ] Dashboard builds cleanly

## Work Log

- 2026-03-12: Created from code review of fix/trace-messages-endpoint branch
