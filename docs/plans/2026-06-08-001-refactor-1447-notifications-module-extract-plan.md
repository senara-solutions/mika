# Plan — refactor(mika#1259): extract notifications/ module (mika#1447)

## Phase 0 — Pin

**A. Foundation §6**:
> `notifications/` — Outbound messages, webhook callbacks, channel-specific formatting. Reads What's Next ranking to decide what to surface.

**B. Sibling-accretion from prior waves**: zero. No accreted methods from #1445 (dashboard_queries) or #1446 (memory). Clean blank-slate firing for the grounding-gate.

**C. Core notifications-domain code surfaced via body-read** — `crates/mika-agent/src/messaging.rs` (494 lines):

| Item | Lines | Purpose |
|---|---|---|
| `pub enum SendOutcome` | 11+ | Result type for message-send operations |
| `pub trait MessageSender: Send + Sync` | 41+ | Abstraction over outbound message delivery |
| `pub struct GatewayMessageSender` | 50+ | Production gateway-side implementation |
| `impl GatewayMessageSender` (new + helpers) | 63+ | Constructor + internal helpers |
| `impl MessageSender for GatewayMessageSender` | 140+ | Trait impl — actual send-call logic |
| `pub struct NoopSender` | 204+ | Test/disabled-mode implementation |
| `impl MessageSender for NoopSender` | 207+ | Trait impl — no-op for tests |
| `fn truncate_for_log` | 216+ | Helper for log-truncation |

**Total: messaging.rs as a single 494-line file** — entire content maps to notifications/ domain.

**D. What stays OUT of #1447**:

- **`tools/send_message.rs`** — the agent-facing `SendMessageTool` belongs to **`tool_execution/`** per §6 ("tool dispatch"). #1450 tool_execution/ grooming will absorb it. notifications/ owns the *send-message logic*; tool_execution/ owns the *tool-call dispatch wrapper* that invokes it.
- **`server/webhook_queue.rs` + `server/handlers.rs` webhook routing** — these are *inbound* webhook handlers (server/ infrastructure layer). §6 notifications/ scope is *outbound* messages + *outbound* webhook callbacks. Inbound = server/, outbound = notifications/.
- **`server/ci_failure_handler.rs` + `server/ci_success_handler.rs` + `server/verdict_handler.rs`** — inbound CI/verdict webhook handlers. Same logic: inbound = server/, outbound = notifications/. notifications/ doesn't absorb these.
- **mika-dev callback-emission code in agent.rs / task_engine/dispatcher.rs** — this is callback-result-emission from the agent loop. It's tightly coupled to agent_loop/ (#1452) execution. Defer to #1452 grooming's scope discussion.

**E. Cross-module dependency check** (grep on messaging.rs):

- messaging.rs imports `mika_common::claude::ToolDefinition` (external crate)
- Uses `tracing` for logging
- No calls to other §6 module methods (no `evidence::*`, no `memory::*`, etc.)
- `tools/send_message.rs` imports `crate::messaging::SendOutcome` — that's a tool_execution/ → notifications/ dependency (one-way; tool_execution/ depends on notifications/ when #1450 extracts; not a notifications/ blocker)

**Pure leaf-with-respect-to-§6**, with one downstream dependent (tool_execution/) that doesn't affect notifications/ extraction.

**F. Tests landscape**: existing tests reference `messaging::SendOutcome`, `messaging::MessageSender`, `messaging::GatewayMessageSender`, `messaging::NoopSender`. After file relocation (`messaging.rs` → `notifications/mod.rs`), tests that import via `mika_agent::messaging::*` need updated to `mika_agent::notifications::*`.

**Same-import-path workaround**: re-export from `crate::messaging` in lib.rs as a deprecated alias, OR rename the import. **Pick rename** — simpler, deletes the legacy path, fits parent #1259's "pure module split, logic identical" framing (no new deprecation aliases).

## Hypothesis (committed)

**Extraction shape**: move `crates/mika-agent/src/messaging.rs` → `crates/mika-agent/src/notifications/mod.rs` (full file relocation). Update all import paths from `crate::messaging::*` → `crate::notifications::*`.

Single-file relocation. Cleanest possible extraction in Wave 2 so far — entire source file maps to one §6 module.

## Approach (committed)

### A. Create the module directory + relocate file

```bash
mkdir -p crates/mika-agent/src/notifications
git mv crates/mika-agent/src/messaging.rs crates/mika-agent/src/notifications/mod.rs
```

(Git rename preserves history.)

### B. Update doc-comment

Top of `crates/mika-agent/src/notifications/mod.rs`:

```rust
//! Outbound messages, webhook callbacks, channel-specific formatting.
//!
//! Owns the `MessageSender` trait abstraction over outbound message delivery,
//! the `GatewayMessageSender` production implementation that drives the
//! mika-gateway HTTP API, and the `NoopSender` test/disabled-mode shim.
//!
//! Per Foundation §6: outbound messages + webhook callbacks. Inbound webhook
//! handling lives in `crate::server` (transport layer); this module owns
//! the outbound side.
//!
//! `crate::tool_execution::send_message` (post-#1450 extraction) wraps the
//! `MessageSender` trait as the agent-facing tool-call API.
```

### C. Update import paths across the codebase

`grep -rn "crate::messaging\|use mika_agent::messaging\|mika_agent::messaging::" crates/ tests/ 2>/dev/null` to find all import sites.

Estimated ~15-30 import-update lines across:
- `crates/mika-agent/src/server/*.rs` (gateway-message-sender used by handlers)
- `crates/mika-agent/src/agent.rs` (agent loop uses MessageSender)
- `crates/mika-agent/src/task_engine/*.rs` (dispatchers may use)
- `crates/mika-agent/src/tools/send_message.rs` (already cited)
- `crates/mika-agent/src/lib.rs` (`pub mod messaging;` → `pub mod notifications;`)
- `crates/mika-agent/tests/**` (test imports)

Each import site: `crate::messaging` → `crate::notifications` (or `mika_agent::messaging` → `mika_agent::notifications`).

### D. Update `lib.rs`

Replace `pub mod messaging;` with `pub mod notifications;`.

### E. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `git log --follow crates/mika-agent/src/notifications/mod.rs` shows preserved history

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/notifications/mod.rs` exists with doc-comment per Foundation §6 (parent AC4).

2. **AC2**: `crates/mika-agent/src/messaging.rs` removed; entire 494-line content relocated to `notifications/mod.rs` via `git mv` (history preserved).

3. **AC3**: All import paths updated from `crate::messaging::*` / `mika_agent::messaging::*` to `crate::notifications::*` / `mika_agent::notifications::*`. Verified by `grep -rn "messaging::" crates/ tests/` returning zero hits (or only docstring mentions).

4. **AC4**: `crates/mika-agent/src/lib.rs` declares `pub mod notifications;` (replaced `pub mod messaging;`) (parent AC4).

5. **AC5**: `cargo test -p mika-agent` passes (parent AC2).

6. **AC6**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

7. **AC7**: No behavior change (parent AC3) — pure file relocation + import rename.

8. **AC8**: `git log --follow crates/mika-agent/src/notifications/mod.rs` shows the same history as the original `messaging.rs` (renamed-not-rewritten).

## Files to change

- `crates/mika-agent/src/messaging.rs` → `crates/mika-agent/src/notifications/mod.rs` (rename via `git mv`)
- Doc-comment at top of new file (updated for §6 framing)
- `crates/mika-agent/src/lib.rs` — module declaration rename
- All call-site imports across crates/mika-agent/src/, crates/mika-agent/tests/, possibly crates/mika-gateway/src/ if it imports any messaging types (verify with grep)

## Out of scope

- `tools/send_message.rs` — belongs to #1450 tool_execution/
- server/ inbound webhook handlers — server/ transport layer, not §6 notifications/
- mika-dev callback-emission from agent loop — defer to #1452 agent_loop/ scope
- Splitting messaging.rs into sender.rs + outcome.rs + etc. — pure relocation, no internal restructuring (parent AC3)

## Risk

Low-medium.
- **Import-update sweep**: 15-30 sites need consistent rename. Mitigated by `grep -rn` after the change to verify zero remaining `messaging::` refs.
- **Test imports**: tests/ directory may have its own imports. Cargo build catches anything left over (compiler error on unresolved path).
- **Cross-crate import**: if `crates/mika-gateway/` imports `mika_agent::messaging::*`, that's a cross-crate update. Mitigated by AC3's grep verification across both crates.

Marginally higher risk than pure-impl-block-relocation (#1445, #1446, #1449) because file-rename + import-sweep touches more sites. But still bounded.

## Test plan

1. `cargo build -p mika-agent` clean
2. `cargo test -p mika-agent --lib` passes
3. `cargo build -p mika-gateway` clean (verify cross-crate imports)
4. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
5. `grep -rn "messaging::\|crate::messaging\|mika_agent::messaging" crates/ tests/` returns zero hits (excluding docstring mentions)
6. `git log --follow crates/mika-agent/src/notifications/mod.rs` shows preserved history

## Implementation order

1. `git mv crates/mika-agent/src/messaging.rs crates/mika-agent/src/notifications/mod.rs`
2. Add the §6 doc-comment to top of `notifications/mod.rs`
3. `pub mod messaging;` → `pub mod notifications;` in `lib.rs`
4. `grep -rn "messaging::\|crate::messaging" crates/ tests/` to find all import sites
5. Sed-replace import paths (verify by re-grep showing zero hits)
6. `cargo build -p mika-agent` — fix any missed imports
7. `cargo build -p mika-gateway` (cross-crate sanity)
8. `cargo test -p mika-agent --lib`
9. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings`
