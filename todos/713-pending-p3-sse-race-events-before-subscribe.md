---
status: pending
priority: p3
issue_id: 713
tags: [code-review, performance]
dependencies: []
---

# SSE race: events sent before client subscribes

## Problem Statement
In `handle_message_stream`, the broadcast channel is created and the processing task spawned BEFORE the SSE stream is returned to the client. The spawned task can emit `StatusUpdate(Working)` before the HTTP response begins streaming, so the client may never see the Working transition.

## Findings
- `crates/mika-agent/src/server/a2a.rs` lines 347-467
- Channel created at 347, spawn at 358, SSE returned at 467
- With buffer=32 and only 2 events currently, unlikely to cause Lagged errors but is a correctness issue

## Proposed Solutions
Defer the spawn until after the SSE response is set up, or use a Notify/oneshot to gate the spawn until the first SSE chunk is flushed.

## Acceptance Criteria
- [ ] Client reliably receives all status transitions including initial Working state
