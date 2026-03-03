---
status: pending
priority: p2
issue_id: 427
tags: [code-review, quality, team-mode, observability]
dependencies: []
---

# Fire-and-forget team message save silently discards errors

## Problem Statement

In `App::send_message_with_thinking()`, user messages are persisted via fire-and-forget `tokio::spawn` with `let _ =` discarding errors. If the database write fails (disk full, DB locked), the message is lost silently with no log entry. Additionally, there's an inconsistency: user messages use fire-and-forget while deliverables use inline await — both discard errors with `let _ =`.

## Findings

- Source: security-sentinel, performance-oracle, pattern-recognition-specialist
- Location: `crates/mika-cli/src/tui/app.rs` line 472-475 (user message fire-and-forget)
- Location: `crates/mika-cli/src/tui/app.rs` line 637 (deliverable inline await)
- Both use `let _ =` to discard errors
- Agent mode does not have this issue because the agent loop itself persists messages

## Proposed Solutions

### Option A: Add tracing::warn! on error (Recommended)
- Replace `let _ =` with `if let Err(e) = ... { tracing::warn!(...) }`
- Apply to both user message and deliverable saves
- **Pros:** Zero UX impact, aids debugging
- **Effort:** Small (2-3 line change per site)
- **Risk:** None

### Option B: Make both writes consistent (inline await)
- Change user message save from fire-and-forget to inline await
- Consistent pattern with deliverable save
- **Pros:** Consistent, guarantees ordering
- **Effort:** Small (remove tokio::spawn wrapper)
- **Risk:** Negligible latency (sub-ms SQLite write)

## Acceptance Criteria

- [ ] Database write failures produce a tracing::warn! log entry
- [ ] Both user and deliverable saves use the same pattern
