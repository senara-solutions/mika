---
status: pending
priority: p3
issue_id: "672"
tags: [code-review, quality, tracing]
dependencies: []
---

# Downgrade Diagnostic Tracing to Debug Level

## Problem Statement

Several `tracing::info!` calls in `messaging.rs:101-107` and `delegate_task.rs` (lines 155, 193, 208, 215) were added during iterative debugging (commits `596f658` and `ddc16bb`). These produce verbose logs in production for routine operations.

## Findings

- `messaging.rs:101-107` logs every outbound send at INFO with `explicit_override` diagnostic field
- `delegate_task.rs` has 6 separate info/warn logs across chat_id resolution and sender creation
- Established pattern: `send_message.rs:57-58` uses `debug!` for per-message sends

## Proposed Solutions

- Downgrade `messaging.rs:101-107` to `debug!`, remove `explicit_override` field
- Keep `warn!` for genuinely unexpected states (corrupt chat_id parse failure)
- Downgrade or remove remaining INFO logs in delegate_task.rs

## Technical Details

- **Affected files:** `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/tools/delegate_task.rs`
- **Effort:** Small (~10 LOC changed)
