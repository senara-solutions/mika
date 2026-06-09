# Plan — mika#1447: Extract `notifications/` into its own module dir

## Context

Parent: mika#1259 (Layer 3 domain refactor — operational-partner project).
Foundation doc: `docs/architecture/operational-partner-frame.md` §6.
Decomposition plan: `docs/plans/2026-06-08-001-meta-1259-decomposition-plan.md`.

**Operational responsibility (Foundation §6):** outbound messages, webhook callbacks, channel-specific formatting.

This is a leaf module in the decomposition sequence — no hard dependencies on other #1259 sub-issues. The ticket is sequenced 4th (after evidence/, dashboard_queries/, memory/) but can land independently.

## Scope — grounded against code

### What moves into `notifications/`

Body-read against the current codebase (branch `main`, commit 88454ee9) identifies the following code that belongs to the notifications operational domain:

| Source file | Lines | What moves | Rationale |
|---|---|---|---|
| `src/messaging.rs` | 495 (all) | Entire file → `notifications/messaging.rs` | Core notification infrastructure: `MessageSender` trait, `GatewayMessageSender`, `NoopSender`, `SendOutcome`, `truncate_for_log`. This IS the notification abstraction. |
| `src/tools/send_message.rs` | 492 (all) | Stays in `tools/send_message.rs` — imports from `notifications::messaging` | The tool is a consumer of the messaging abstraction, not the abstraction itself. Tool registration lives in `tools/mod.rs`; moving the file would disrupt that pattern with no benefit. |
| `src/teams/notification.rs` | 304 (all) | Stays in `teams/notification.rs` — imports from `notifications::messaging` if needed | Team-run completion formatting is team-domain logic that happens to produce notification text. Its primary dependency is `teams::types::{RunStatus, TeamRun}` and `db::TeamRunRow`, not the messaging channel. Moving it would create a circular dependency (notifications → teams types ← teams → notifications). |

### What gets factored out (new code in `notifications/`)

| New artifact | Est. lines | Content | Consumers |
|---|---|---|---|
| `notifications/mod.rs` | ~15 | Module doc-comment (AC4), re-exports | All consumers of `messaging.rs` |
| `notifications/helpers.rs` | ~30 | `send_notification()` helper — factored from duplicated implementations in `server/verdict_handler.rs:1136-1146` and `server/ci_failure_handler.rs:582-595` | `server/verdict_handler.rs`, `server/ci_failure_handler.rs` |

### What does NOT move (grounded exclusions)

| File | Lines | Why it stays | Per Foundation §6 owner |
|---|---|---|---|
| `src/webhook_dispatch.rs` | 185 | Dispatch-gating predicates (`is_unauthorized_webhook_dispatch`, `is_ready_label_dispatch_marker`). These are policy/planning decisions, not notification infrastructure. | `planning/` |
| `src/server/webhook_queue.rs` | 343 | In-memory webhook deferral queue. Server-side request sequencing, not outbound delivery. | `server/` or `planning/` |
| `src/server/verdict_handler.rs` | 2024 | Structural handler logic (verdict parsing, dispatch, merge). Only the `send_notification` helper (~10 lines) is notification-domain; the rest is server/planning. | `server/` |
| `src/server/ci_failure_handler.rs` | 922 | Structural handler logic (CI failure pre-digest, circuit breaker). Same pattern — `send_notification` factored out, handler stays. | `server/` |
| `src/server/ci_success_handler.rs` | 698 | Merge re-evaluation logic. Minimal notification usage. | `server/` |
| `src/teams/notification.rs` | 304 | Team-run completion formatting — depends on `teams::types`, not on the messaging channel. | `teams/` |

### Why `tools/send_message.rs` stays in `tools/`

The decomposition plan (§D "notifications/") estimated ~1,000 LoC and named "agent.rs + messaging.rs + server/" as sources. Body-read shows:

1. `send_message.rs` is a **tool implementation** — it implements the `Tool` trait, lives in `tools/`, is registered in `tools/mod.rs::default_tools()`. Moving it to `notifications/` would break the uniform tool-registration pattern (all tools live in `tools/`).
2. The tool **consumes** `messaging::MessageSender` via `ctx.message_sender` — it's a client, not part of the abstraction.
3. The `tools/` directory already has a clear ownership boundary. Splitting one tool out creates precedent confusion for future tool ownership.

This is a divergence from Foundation §6's "channel-specific formatting" framing. The formatting happens in the `MessageSender` implementations (which do move), not in the tool dispatch code.

## Implementation steps

### Step 1: Create `notifications/mod.rs` with re-exports

Create `crates/mika-agent/src/notifications/mod.rs`:

```rust
//! Outbound messages, webhook callbacks, and channel-specific formatting.
//!
//! Owns the `MessageSender` trait and its implementations (`GatewayMessageSender`,
//! `NoopSender`), the `SendOutcome` delivery result type, and shared notification
//! helpers used by server handlers.

mod messaging;
pub mod helpers;

pub use messaging::{
    GatewayMessageSender, MessageSender, NoopSender, SendOutcome,
    truncate_for_log,
};
```

### Step 2: Move `messaging.rs` → `notifications/messaging.rs`

1. `git mv src/messaging.rs src/notifications/messaging.rs`
2. Update internal module path: change `pub mod messaging;` visibility if needed (it becomes `mod messaging;` inside `notifications/mod.rs`, with pub re-exports).
3. Make `truncate_for_log` `pub(crate)` (currently private, but the re-export makes it visible through `notifications::truncate_for_log`; alternatively keep private and only expose through `helpers::send_notification`).

### Step 3: Create `notifications/helpers.rs` — factored `send_notification`

Extract the duplicated `send_notification` async fn from `server/verdict_handler.rs:1136-1146` and `server/ci_failure_handler.rs:582-595` into a shared helper:

```rust
//! Shared fire-and-forget notification helpers for server handlers.

use std::sync::Arc;
use tracing::warn;

use super::{MessageSender, SendOutcome};

/// Send a notification message via the configured sender.
///
/// Fire-and-forget: logs warnings on failure but never returns an error.
/// Used by structural handlers (verdict, CI) for operator notifications.
pub(crate) async fn send_notification(
    sender: &Arc<dyn MessageSender>,
    message: &str,
    handler_name: &str,
) {
    match sender.send(message).await {
        Ok(SendOutcome::Delivered) => {}
        Ok(SendOutcome::Failed { reason }) => {
            warn!(reason = %reason, handler = handler_name, "notification delivery failed");
        }
        Ok(SendOutcome::NoChannel) => {
            warn!(handler = handler_name, "notification skipped — no reply channel (chat_id=0)");
        }
        Err(e) => {
            warn!(error = %e, handler = handler_name, "notification sender error");
        }
    }
}
```

The parameterized `handler_name` replaces the handler-specific string literals in the current duplicated implementations, improving log grep-ability.

### Step 4: Update `lib.rs` module declaration

Replace `pub mod messaging;` with `pub mod notifications;` in `crates/mika-agent/src/lib.rs`.

### Step 5: Update all import sites

All consumers of `crate::messaging::*` switch to `crate::notifications::*`. Grounded list of affected files (13 call sites across 10 files):

| File | Current import | New import |
|---|---|---|
| `src/agent.rs:18` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/tools/mod.rs:66` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/tools/send_message.rs:7` | `use crate::messaging::SendOutcome` | `use crate::notifications::SendOutcome` |
| `src/tools/send_message.rs:122` | `use crate::messaging::{MessageSender, SendOutcome}` | `use crate::notifications::{MessageSender, SendOutcome}` |
| `src/tools/run_team.rs:13` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/server/mod.rs:70` | `use crate::messaging::{GatewayMessageSender, MessageSender}` | `use crate::notifications::{GatewayMessageSender, MessageSender}` |
| `src/server/handlers.rs:16` | `use crate::messaging::{GatewayMessageSender, MessageSender}` | `use crate::notifications::{GatewayMessageSender, MessageSender}` |
| `src/server/verdict_handler.rs:28` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` + use `notifications::helpers::send_notification` |
| `src/server/ci_failure_handler.rs:33` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` + use `notifications::helpers::send_notification` |
| `src/server/ci_success_handler.rs:38` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/task_engine/dispatcher.rs:26` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/task_engine/dispatcher.rs:1829` | `use crate::messaging::{MessageSender, SendOutcome}` | `use crate::notifications::{MessageSender, SendOutcome}` |
| `src/task_engine/engine.rs:1189` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |

Additionally, the inline `crate::messaging::SendOutcome::*` paths in `verdict_handler.rs:1138-1142` and `ci_failure_handler.rs:584-588` are replaced by the shared `notifications::helpers::send_notification` call.

### Step 6: Remove duplicated `send_notification` from server handlers

After step 3+5, delete:
- `server/verdict_handler.rs:1136-1146` (the local `send_notification` fn)
- `server/ci_failure_handler.rs:582-595` (the local `send_notification` fn)

Replace call sites with `crate::notifications::helpers::send_notification(sender, message, "verdict_handler")` and `crate::notifications::helpers::send_notification(sender, message, "ci_failure_handler")` respectively.

### Step 7: Verify

1. `cargo test -p mika-agent` — all existing tests pass unchanged
2. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — clean
3. `cargo build` — full workspace build succeeds

## Acceptance criteria traceability

| AC | How satisfied |
|---|---|
| AC1: `notifications/mod.rs` with doc-comment | Step 1 — one-paragraph doc-comment naming operational responsibility |
| AC2: `cargo test -p mika-agent` passes | Step 7 — no behavior change, tests verify |
| AC3: No behavior change — pure module split | Steps 2-6 move code without modifying logic. The only new code is `helpers::send_notification` which replaces two identical copies. |
| AC4: `lib.rs` declares new module | Step 4 |
| AC5: `cargo clippy` clean | Step 7 |

## LoC accounting

- **Moving into `notifications/`:** ~495 (messaging.rs) + ~45 (new mod.rs + helpers.rs) = ~540 LoC
- **Net new code:** ~45 lines (mod.rs + helpers.rs). The `send_notification` helper replaces ~26 lines of duplicated code across two files (net +19 lines).
- **Net change to `agent.rs`/`db.rs`:** 0 lines moved from either. This module's extraction is from standalone files, not from the two monoliths targeted by parent #1259's AC5.

This is the smallest of the 9 sub-issues (decomposition estimated ~1,000 LoC; actual is ~540). The discrepancy is grounded: the estimate included `tools/send_message.rs` and `teams/notification.rs` which stay in their current homes per the analysis above.

## Risk

**Low.** Pure mechanical refactor:
- All changes are import-path updates (find-and-replace) plus one file move
- The new `helpers::send_notification` is a direct extraction of duplicated code — no new logic
- No cross-crate changes (all within `mika-agent`)
- No DB schema changes
- No behavior changes

## Out of scope

- Other #1259 sub-issues (evidence/, tool_execution/, memory/, etc.)
- Cross-module interface redesign (this is pure relocation per AC3)
- Moving `webhook_dispatch.rs` (belongs in `planning/`, a different sub-issue)
- Moving `teams/notification.rs` (stays in `teams/` — see exclusion rationale above)
- Moving `tools/send_message.rs` (stays in `tools/` — see exclusion rationale above)
