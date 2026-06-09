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
| `src/messaging.rs` | 495 (all) | Entire file → `notifications/mod.rs` via `git mv` (history preserved) | Core notification infrastructure: `MessageSender` trait, `GatewayMessageSender`, `NoopSender`, `SendOutcome`, `truncate_for_log`. This IS the notification abstraction. |
| `src/tools/send_message.rs` | 492 (all) | Stays in `tools/send_message.rs` — imports from `notifications::*` | The tool is a consumer of the messaging abstraction, not the abstraction itself. Tool registration lives in `tools/mod.rs`; moving the file would disrupt that pattern with no benefit. |
| `src/teams/notification.rs` | 304 (all) | Stays in `teams/notification.rs` — imports from `notifications::*` if needed | Team-run completion formatting is team-domain logic that happens to produce notification text. Its primary dependency is `teams::types::{RunStatus, TeamRun}` and `db::TeamRunRow`, not the messaging channel. Moving it would create a circular dependency (notifications → teams types ← teams → notifications). |

### Extraction shape: single-file `git mv`

This is a **pure relocation** matching the GROOMED extraction shape: `messaging.rs → notifications/mod.rs` via a single `git mv`, preserving file history. No internal restructuring — no submodule split, no new files beyond `mod.rs` (which IS the renamed `messaging.rs`). The doc-comment is prepended to the top of the moved file.

**No `helpers.rs`, no DRY extraction.** The duplicated `send_notification` implementations in `server/verdict_handler.rs` and `server/ci_failure_handler.rs` remain in place, unchanged. Factoring them into a shared helper is a good idea but constitutes a DRY refactoring beyond the "pure module split, logic identical" AC — it belongs in a follow-up ticket, not this relocation.

### What does NOT move (grounded exclusions)

| File | Lines | Why it stays | Per Foundation §6 owner |
|---|---|---|---|
| `src/webhook_dispatch.rs` | 185 | Dispatch-gating predicates (`is_unauthorized_webhook_dispatch`, `is_ready_label_dispatch_marker`). These are policy/planning decisions, not notification infrastructure. | `planning/` |
| `src/server/webhook_queue.rs` | 343 | In-memory webhook deferral queue. Server-side request sequencing, not outbound delivery. | `server/` or `planning/` |
| `src/server/verdict_handler.rs` | 2024 | Structural handler logic (verdict parsing, dispatch, merge). Its local `send_notification` helper stays in-file (see extraction shape above). | `server/` |
| `src/server/ci_failure_handler.rs` | 922 | Structural handler logic (CI failure pre-digest, circuit breaker). Its local `send_notification` helper stays in-file. | `server/` |
| `src/server/ci_success_handler.rs` | 698 | Merge re-evaluation logic. Minimal notification usage. | `server/` |
| `src/teams/notification.rs` | 304 | Team-run completion formatting — depends on `teams::types`, not on the messaging channel. | `teams/` |

### Why `tools/send_message.rs` stays in `tools/`

The decomposition plan (§D "notifications/") estimated ~1,000 LoC and named "agent.rs + messaging.rs + server/" as sources. Body-read shows:

1. `send_message.rs` is a **tool implementation** — it implements the `Tool` trait, lives in `tools/`, is registered in `tools/mod.rs::default_tools()`. Moving it to `notifications/` would break the uniform tool-registration pattern (all tools live in `tools/`).
2. The tool **consumes** `messaging::MessageSender` via `ctx.message_sender` — it's a client, not part of the abstraction.
3. The `tools/` directory already has a clear ownership boundary. Splitting one tool out creates precedent confusion for future tool ownership.

This is a divergence from Foundation §6's "channel-specific formatting" framing. The formatting happens in the `MessageSender` implementations (which do move), not in the tool dispatch code.

## Implementation steps

### Step 1: Move `messaging.rs` → `notifications/mod.rs` (single `git mv`)

1. `mkdir -p crates/mika-agent/src/notifications/`
2. `git mv crates/mika-agent/src/messaging.rs crates/mika-agent/src/notifications/mod.rs`
3. Prepend the doc-comment to the top of `notifications/mod.rs`:

```rust
//! Outbound messages, webhook callbacks, and channel-specific formatting.
//!
//! Owns the `MessageSender` trait and its implementations (`GatewayMessageSender`,
//! `NoopSender`), the `SendOutcome` delivery result type, and log-truncation utilities.
```

No visibility changes needed — the file was `pub mod messaging;` and becomes `pub mod notifications;` with the same public exports. `truncate_for_log` retains its current visibility (pub(crate) if already so, private otherwise).

### Step 2: Update `lib.rs` module declaration

Replace `pub mod messaging;` with `pub mod notifications;` in `crates/mika-agent/src/lib.rs`.

### Step 3: Update all import sites

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
| `src/server/verdict_handler.rs:28` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/server/ci_failure_handler.rs:33` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/server/ci_success_handler.rs:38` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/task_engine/dispatcher.rs:26` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |
| `src/task_engine/dispatcher.rs:1829` | `use crate::messaging::{MessageSender, SendOutcome}` | `use crate::notifications::{MessageSender, SendOutcome}` |
| `src/task_engine/engine.rs:1189` | `use crate::messaging::MessageSender` | `use crate::notifications::MessageSender` |

No changes to `verdict_handler.rs` or `ci_failure_handler.rs` beyond the import path — their local `send_notification` functions remain untouched.

### Step 4: Verify

1. `cargo test -p mika-agent` — all existing tests pass unchanged
2. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — clean
3. `cargo build` — full workspace build succeeds

## Acceptance criteria traceability

| AC | How satisfied |
|---|---|
| AC1: `notifications/mod.rs` with doc-comment | Step 1 — doc-comment prepended to the `git mv`-ed file, naming operational responsibility |
| AC2: `cargo test -p mika-agent` passes | Step 4 — no behavior change, tests verify |
| AC3: No behavior change — pure module split, logic identical | Steps 1-3 are a single `git mv` + import sweep. Zero new logic, zero modified logic. Server handler `send_notification` functions are untouched. |
| AC4: `lib.rs` declares new module | Step 2 |
| AC5: `cargo clippy` clean | Step 4 |

## LoC accounting

- **Moving into `notifications/`:** ~495 (messaging.rs via `git mv` → `notifications/mod.rs`)
- **Net new code:** ~4 lines (doc-comment prepended to `mod.rs`)
- **Net change to `agent.rs`/`db.rs`:** 0 lines moved from either. This module's extraction is from a standalone file, not from the two monoliths targeted by parent #1259's AC5.

This is the smallest of the 9 sub-issues (decomposition estimated ~1,000 LoC; actual is ~495). The discrepancy is grounded: the estimate included `tools/send_message.rs` and `teams/notification.rs` which stay in their current homes per the analysis above.

## Risk

**Low.** Pure mechanical refactor:
- Single `git mv` preserving file history + import-path sweep (find-and-replace)
- No new files beyond the renamed `mod.rs`
- No cross-crate changes (all within `mika-agent`)
- No DB schema changes
- No behavior changes — no logic added, modified, or removed

## Out of scope

- Other #1259 sub-issues (evidence/, tool_execution/, memory/, etc.)
- Cross-module interface redesign (this is pure relocation per AC3)
- DRY extraction of duplicated `send_notification` from server handlers (good follow-up, but exceeds "pure module split" AC — see revision history)
- Moving `webhook_dispatch.rs` (belongs in `planning/`, a different sub-issue)
- Moving `teams/notification.rs` (stays in `teams/` — see exclusion rationale above)
- Moving `tools/send_message.rs` (stays in `tools/` — see exclusion rationale above)

## Revision history

- rev 2 (2026-06-09): addressed F1 by removing `helpers.rs` and the `send_notification` DRY extraction — the plan is now a pure relocation matching issue AC3 ("No behavior change — pure module split, logic identical") and parent #1259 AC3. The DRY extraction is noted as a good follow-up in Out of Scope. Addressed F2 by reverting to the GROOMED extraction shape: single `git mv messaging.rs → notifications/mod.rs` (one file, history preserved) instead of a three-file `mod.rs` + `messaging.rs` + `helpers.rs` layout. Per review-guide.md § KISS, the single-file layout is simpler and already ratified. F3 is moot — no extraction to verify since `helpers.rs` was removed.
