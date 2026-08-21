---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
origin: github-issue senara-solutions/mika#1758
created: 2026-08-20
---

# feat(server): Emit TaskEventFrame from task_engine transition sites (#1758)

## Summary

mika#1732 shipped the wire (enum, channel, SSE route, docs) for the task-event live stream but left the emission plumbing for a follow-up — this ticket. Without emission, the wire is dormant: subscribers connect, the stream stays silent, mika#1727 (TUI status pane) cannot land. This plan implements the emission by threading a shared per-process `TaskEventsChannel` handle into `AsyncDatabase` and firing `TaskEventFrame` variants at the appropriate wrapper methods AFTER each successful DB write. The centralised-at-the-wrapper approach is the architect-endorsed shape from the mika#1732 grooming pass (`4ac4247a-0d38-4ac2-a8fe-6196fe31bf9f`) and covers every transition site in one write per variant, versus scattering emissions across `dispatcher.rs` + `engine.rs` + tool + handler sites.

Time-bound commitment: 14 days from mika#1732 merge or escalate to P1 (load-bearing for mika#1727).

---

## Problem Frame

**What breaks:** `TaskEventsChannel::broadcast_frame` is never called. `dashboard/tasks/stream` opens, keeps the connection alive, and emits zero frames on any lifecycle transition. Consumers cannot observe live task state without polling `GET /api/v1/tasks`. mika#1727's TUI status pane is stalled behind this ticket.

**Why the wire went dormant:** mika#1732 shipped as a "wire-first split" — the enum, channel primitive, and SSE handler landed, but the actual emission at transition sites was moved to a follow-up ticket to keep the wire PR reviewable. The follow-up (this ticket) landed on the 14-day time-bound with the escalate-to-P1 clause per the mika-arch first-pass review to prevent the `permissions_stream.rs`-dead-code-for-months anti-pattern.

**Six lifecycle transitions to emit:** `TaskCreated` (row inserted), `TaskClaimed` (transitioned to `in_progress` via `claim_and_fire_task`), `TaskCompleted` (`update_task_completed` succeeded), `TaskFailed` (`update_task_failed` succeeded), `TaskDelivered` (callback delivery via `mark_task_delivered`), `TaskCancelled` (`cancel_task` or `update_task_status(id, "cancelled")`). Plus `OverflowMarker` which the SSE handler already emits under `Lagged` — no emission-side change needed for the marker.

**Bearing (from ticket):** "Emit AFTER the successful DB write. Fire-and-forget: broadcast errors do NOT fail the caller's DB write." Per-process single-tenant broadcast is correct for v1; frames carry `agent_id` for client-side filter.

---

## Requirements

- **R1.** All six emission variants (`TaskCreated` / `TaskClaimed` / `TaskCompleted` / `TaskFailed` / `TaskDelivered` / `TaskCancelled`) are emitted exactly once per real DB-observed transition. `OverflowMarker` continues to originate from the SSE handler's `Lagged` path (no change).
- **R2.** Emission runs AFTER the DB write commits. When the underlying `db.*` call returns `Ok(true)` (a real transition happened) or `Ok(id)` (row created), the frame fires. When the call returns `Ok(false)` (no-op — status guard blocked it) or `Err(_)`, no frame is emitted.
- **R3.** Emission is fire-and-forget: `broadcast_frame` returning `false` (zero subscribers, per `broadcast::Sender::send` returning `Err`) is not an error and does NOT fail the caller's DB write path. No panic on serde/channel failure.
- **R4.** The channel handle is threaded via `AsyncDatabaseInner` behind a `OnceLock`, attached once per process at server startup. Tests, CLI, and any non-server caller construct `AsyncDatabase` unchanged — an unset channel means silent no-op emission (correct for those contexts).
- **R5.** Integration test: a synthetic task lifecycle (`create → claim → complete`, and separately `create → claim → fail`, and `create → claim → complete → mark_delivered`, and `create → cancel`) produces the exact expected frame sequence for a live subscriber.
- **R6.** Load-shape test: sustained emission does not exhibit `OverflowMarker` under normal single-subscriber load. The mika#1732 ticket AC5's full 30-second 100-events/sec x 10-subscribers soak is left as a manual verification note (below) because CI cannot reliably assert throughput; the deterministic invariant that we CAN gate on is "no `OverflowMarker` fires when the emission rate does not exceed the drain rate."
- **R7.** Stub consumer per mika#1732 AC4: one `mika-cli` subcommand (`mika dev tasks-stream`) subscribes to `GET /api/v1/dashboard/tasks/stream` and logs frames as they arrive. Not integrated into the TUI (mika#1727 handles that).
- **R8.** The 14-day-or-P1 commitment is honoured by shipping this PR before the deadline. When merged, the wire graduates from dormant to live and mika#1727 unblocks.

---

## Product Contract preservation

No prior brainstorm/Product Contract exists — this plan is `product_contract_source: ce-plan-bootstrap`. The ticket body is the authoritative product input; its acceptance criteria are transcribed verbatim into `## Acceptance criteria` below.

---

## Key Technical Decisions

### KTD1. Emit from the `AsyncDatabase` wrapper methods, not from dispatcher/engine/tool call sites

**Choice:** Add emission points to the six `AsyncDatabase` wrapper methods that shepherd every task lifecycle transition. Not at dispatcher/engine/tool/handler call sites.

**Rationale:**
- Ticket architect note explicitly suggests wrapper-centralisation: "consider emitting from the `AsyncDatabase::create_task_async` wrapper for centralization."
- Every real transition passes through `AsyncDatabase` — no call site can bypass. Callsites listed in the ticket body (dispatcher.rs:1702, engine.rs:837, etc.) are all indirect callers of these wrappers.
- Adding at wrappers is one write per variant; adding at call sites is ~15 writes with the same variant duplicated. DRY win, orthogonality win (per `docs/architecture/review-guide.md`), audit-clarity win.
- Tests that construct `AsyncDatabase` without attaching a channel naturally get zero emissions — no test needs to be updated to silence broadcast noise.

**Alternatives rejected:**
- Emit from every call site: ~15 emission points, high drift risk (missed sites when new callers land — same class as the mika#582/#630/#801 "add write, forget to update guard" family). Also forces the frame construction shape to be duplicated.
- Emit from the sync `Database` layer: `Database::create_task` is `!Send` and returns before the transaction commits (in some paths); the async wrapper is the correct commit boundary.
- Emit from an event bus at the tool layer: `create_task` is called from tools, dispatcher, engine, HTTP handlers, and cron — the tool layer is only one of five callers.

### KTD2. Channel handle via `OnceLock<Arc<TaskEventsChannel>>` on `AsyncDatabaseInner`

**Choice:** Add `task_events_channel: OnceLock<Arc<TaskEventsChannel>>` to `AsyncDatabaseInner`. A single `set_task_events_channel(&self, channel: Arc<TaskEventsChannel>)` method attaches at server startup; all clones via `with_agent()` share the same `Inner` and see the same channel.

**Rationale:**
- `OnceLock` is zero-alloc atomic read on the hot path (per-transition).
- Attach-once semantics matches reality: one `TaskEventsChannel` per server process.
- Absent-when-unset is the correct default for tests / CLI / any non-server caller.
- No breaking change to `AsyncDatabase::new` / `new_with_agent` / `open` signatures — additive.

**Alternatives rejected:**
- Passing `Option<Arc<TaskEventsChannel>>` into `AsyncDatabase::new_with_agent`: breaks ~40 test call sites that construct `AsyncDatabase` directly, and threads the channel through `init_agent`'s signature. `OnceLock` avoids both.
- `RwLock<Option<...>>`: overkill for a set-once field; adds contention that `OnceLock` avoids.
- Per-outer-clone field: would require passing the channel through every `with_agent()` call — `with_agent` is called in many hot paths (per-turn), forcing an argument bloom.

### KTD3. Emit AFTER the DB call returns, gated on `Ok(true)` / `Ok(id)`

**Choice:** Each wrapper method awaits the DB call, checks the result, and only fires the frame when the DB write actually transitioned the row (`Ok(true)` for guarded UPDATEs, `Ok(id)` for INSERTs). `Ok(false)` (no-op) and `Err(_)` never fire.

**Rationale:**
- Preserves the "frame captures the AFTER state" contract from the ticket.
- Guarded UPDATEs (`update_task_completed`'s `status IN ('pending', 'in_progress')` guard, `update_task_failed`'s terminal-state guard, `cancel_task`'s cancellable-state guard, `mark_task_delivered`'s `status='completed' AND delivered_at IS NULL` guard, `claim_and_fire_task`'s atomic claim guard) already encode "did anything change?" in their return type. Reusing that return type prevents phantom frames for no-op calls.
- `Err(_)` skipping is correct: the caller sees an error, no state change, no observer should see a fake success frame.

### KTD4. Include `TaskCancelled` on both dedicated (`cancel_task`) and generic (`update_task_status(id, "cancelled")`, `update_manual_task_status(id, "cancelled")`) paths

**Choice:** Emit `TaskCancelled` from three wrapper methods: `cancel_task`, `update_task_status` when `status == "cancelled"`, `update_manual_task_status` when `new_status == "cancelled"`. Each emits at most once per real DB-observed transition; the paths are mutually exclusive at any given transition site.

**Rationale:**
- `cancel_task` is the primary operator-cancel path (`cancel_task_and_kill` in `process_kill.rs` and the CLI/handler paths).
- `update_task_status` is called with `"cancelled"` by `teams/engine.rs:1329` (team-parent cancellation) — a legitimate real-code path.
- `update_manual_task_status` is the tool-layer wrapper for status writes; used by the `update_task_status` tool (agent-authored) and could carry `"cancelled"`.
- Missing any of the three paths creates a hole where the frame doesn't fire on a legitimate cancel transition.

**Alternatives rejected:**
- Emit only from `cancel_task`: misses teams/engine.rs path.
- Emit `TaskCancelled` unconditionally from `update_task_status` regardless of the new status: wrong — the method also carries other status transitions.

### KTD5. `promote_task_completed` (failed→completed retry-promoter) fires `TaskCompleted`

**Choice:** Emit `TaskCompleted` from `promote_task_completed` too — this is `failed → completed` (a semantic completion).

**Rationale:** From an observer's standpoint, "task X reached completed" is the semantic invariant, regardless of the intermediate `failed` state. The retry-promoter (mika#958) exists precisely to correct a false-fail. Not emitting here would hide a real state transition from the wire.

### KTD6. `TaskCreated` fields — extract from the `NewTask` before the DB thread hop

**Choice:** In `AsyncDatabase::create_task`, clone the emission-relevant fields (`trigger_type`, `action_type`, `label`, `parent_task_id`, `agent_id`) from the incoming `NewTask` BEFORE moving it into the DB-thread closure. Timestamp = `crate::timestamp::now()` captured at the same moment. On successful `Ok(id)`, emit `TaskCreated` with the captured fields.

**Rationale:**
- `NewTask` moves into the closure — attempting to read fields after would need another DB read.
- Capturing `now()` at the emit point is off by ≤1ms vs the DB-committed `created_at`; the frame is a UI hint, exact-match against DB is not required.
- **agent_id source is `task.agent_id`, NOT `self.agent_id`.** `Database::create_task`'s INSERT uses `task.agent_id`; callers on shared handles legitimately pass a different `NewTask.agent_id` (e.g., `teams/engine.rs` writes child tasks with the assigned team member's agent id, not the orchestrator's). Correlating the frame to `GET /api/v1/tasks/{task_id}` must show a consistent view — the frame's agent_id must match the row on disk.

### KTD10. `update_task_status(id, "cancelled")` requires a prior-state pre-fetch

**Choice:** Unlike the other cancel wrappers (`cancel_task`, `update_manual_task_status`), `Database::update_task_status` is an unguarded `UPDATE ... WHERE id = ?` returning `Ok(())` regardless of whether the row existed or the status actually changed. To honour the "emit only on real transitions" invariant, the wrapper pre-fetches the task via `get_task_unscoped` when a cancel emit would otherwise fire, and only emits when the row exists AND its prior status was NOT `cancelled`. Fail-closed on the read path: a pre-fetch error skips the emit rather than firing spuriously.

**Rationale:** Cheapest surgical fix that doesn't widen the `Database::update_task_status` signature (42 callers). Adds one DB read per cancel-via-status call — a rare path used primarily by `teams/engine.rs` cancelling a team-parent task. Prevents a spurious frame on any redundant re-cancel (idempotent second call), unknown-id call, or leftover call after the row has already been cancelled by another path.

### KTD11. Recurring-task cleanup path stays silent on v1

**Choice:** `AsyncDatabase::cancel_recurring_task_by_label` cancels 0..N rows keyed by (agent_id, label) but the underlying DB call does not return the affected task ids. Emitting per-row `TaskCancelled` frames would require a pre-fetch (`SELECT id FROM tasks WHERE ...`) that doubles the DB round-trip on the cleanup path. Deferred to a follow-up that widens the DB signature to return the affected ids. The wire stays silent on this cleanup transition for v1; the operator paths (`cancel_task`, `update_manual_task_status`) do emit.

### KTD7. Fire-and-forget wrapper: `emit_task_event(&self, TaskEventFrame)` on `AsyncDatabase`

**Choice:** Add a private `fn emit_task_event(&self, frame: TaskEventFrame)` method on `AsyncDatabase` that reads `self.inner.task_events_channel.get()`, calls `.broadcast_frame(frame)` if present, ignores the returned bool. No `.await`, no `Result`.

**Rationale:**
- Broadcast is synchronous under the hood (`tokio::sync::broadcast::Sender::send` is non-blocking).
- Zero perf overhead when channel is unset (single atomic load returns `None`).
- Single choke point for panic-safety and for the "fire-and-forget" invariant.

### KTD8. Stub consumer: `mika dev tasks-stream` in `mika-cli`

**Choice:** Add a subcommand under `mika dev` (existing dev/hidden namespace or new small command) that opens the SSE stream and prints one frame per line to stdout. Not integrated into the interactive TUI.

**Rationale:** Satisfies mika#1732 AC4 verbatim ("one-file `mika-cli` client that logs frames as they arrive"). Placing it under `mika dev` keeps it out of the customer-facing CLI surface (this is a diagnostic).

### KTD9. Load-test discipline: assert the deterministic invariant, note the throughput soak

**Choice:** The unit-test suite gates on "no `OverflowMarker` under normal load" and "6 sequential lifecycle frames delivered in order." The 30-sec 100-eps × 10-subscribers soak from mika#1732 AC5 is documented as a manual `MIKA_TASK_STREAM_SOAK=1 cargo test -p mika-agent --test task_event_stream_load -- --ignored` invocation with a `#[ignore]` attribute so operators can run it on demand but CI does not gate on it (throughput is non-deterministic under CI runner load).

**Rationale:** CI throughput asserts are flake vectors (mika#1244-adjacent). The deterministic invariants (order, count, no overflow under matched drain) are what stability requires. The soak invocation preserves the ability to verify but does not brittle-gate CI.

---

## Definition of Done

- [ ] `AsyncDatabaseInner` carries a `OnceLock<Arc<TaskEventsChannel>>`; `set_task_events_channel(&self, ...)` attaches once.
- [ ] `AsyncDatabase::emit_task_event(&self, TaskEventFrame)` fires the frame if the channel is set, silent no-op otherwise.
- [ ] Six wrapper methods emit their frame AFTER the DB write returns the transition-happened signal:
  - [ ] `create_task` → `TaskCreated` on `Ok(id)`
  - [ ] `claim_and_fire_task` → `TaskClaimed` on `Ok(true)`
  - [ ] `update_task_completed` → `TaskCompleted` on `Ok(true)` (with `truncate_preview`d result)
  - [ ] `update_task_failed` → `TaskFailed` on `Ok(true)` (with `truncate_preview`d error)
  - [ ] `mark_task_delivered` → `TaskDelivered` on `Ok(true)`
  - [ ] `cancel_task` → `TaskCancelled` on `Ok(true)` (with reason `"cancelled_via_cancel_task"`)
- [ ] Additional cancel paths emit `TaskCancelled`:
  - [ ] `update_task_status(id, "cancelled")` on `Ok(())`
  - [ ] `update_manual_task_status(id, ..., "cancelled")` on `Ok(Some(_))`
- [ ] `promote_task_completed` emits `TaskCompleted` on `Ok(true)` (mika#958 promotion path).
- [ ] `server::mod::run_server` and `AppState::resolve_agent` attach the shared `AppState.task_events_channel` to each agent's `AsyncDatabase` after construction.
- [ ] Integration test: `tests/task_engine_event_stream.rs` (or crate-internal) asserts frame sequence and content for four lifecycle shapes: `create→claim→complete`, `create→claim→fail`, `create→claim→complete→mark_delivered`, `create→cancel`.
- [ ] Unit test: `broadcast_frame` fire-and-forget invariant — DB write succeeds even when the channel has zero subscribers or is not set.
- [ ] Unit test: guarded no-op DB returns (`Ok(false)`) do NOT emit frames.
- [ ] `mika dev tasks-stream` subcommand implemented; smoke-tested against a live mika-spirit.
- [ ] `cargo build --release`, `cargo test`, `cargo clippy`, `cargo fmt --check` all clean.
- [ ] PR body carries `Closes #1758` and calls out the 14-day-or-P1 commitment satisfaction.

---

## Acceptance criteria

Transcribed verbatim from the issue body:

- [ ] **AC1** — All 6 `TaskEventFrame` variants emitted from the appropriate transition sites (catalog above).
- [ ] **AC2** — Emission is AFTER successful DB write. Fire-and-forget: broadcast errors do NOT fail the caller's DB write.
- [ ] **AC3** — Integration test with a synthetic task lifecycle: create → claim → complete → deliver produces the corresponding frame sequence.
- [ ] **AC4** — Load test per mika#1732 ticket AC5: 10 concurrent subscribers + 100 events/sec sustained for 30s. `OverflowMarker` fires only under overflow, not under normal load.
- [ ] **AC5** — Stub consumer per mika#1732 ticket AC4: one-file `mika-cli` client that logs frames as they arrive. Not integrated into TUI proper (that's mika#1727 closing PR).

---

## Verification Contract

**Test class**: unit + integration (in-crate).

**Deterministic invariants gated by CI:**
1. `create_task` returns `Ok(id)` → subscriber receives `TaskCreated { task_id: id, agent_id: "mika", … }` frame within 1 sec.
2. `claim_and_fire_task(id)` returns `Ok(true)` → subscriber receives `TaskClaimed { task_id: id, … }`. `Ok(false)` → no frame.
3. `update_task_completed(id, Some("result string"))` returns `Ok(true)` → `TaskCompleted { task_id: id, result_preview: Some("result string"), … }`. `Ok(false)` (already terminal) → no frame.
4. `update_task_failed(id, "error string")` returns `Ok(true)` → `TaskFailed { task_id: id, error_preview: Some("error string"), … }`.
5. `mark_task_delivered(id)` returns `Ok(true)` → `TaskDelivered`. `Ok(false)` (already delivered) → no frame.
6. `cancel_task(id)` returns `Ok(true)` → `TaskCancelled`. `Ok(false)` (not in cancellable state) → no frame.
7. Full lifecycle `create → claim → complete → mark_delivered` produces exactly 4 frames in order (no extras).
8. Full lifecycle `create → cancel` produces exactly 2 frames in order.
9. Fire-and-forget: DB write succeeds and returns `Ok(...)` even when zero subscribers exist.
10. Absent channel (never attached): DB write succeeds; no panic; no frame observable.

**Manual verification (documented, not CI-gated):**
- `MIKA_TASK_STREAM_SOAK=1 cargo test -p mika-agent task_event_stream_soak -- --ignored --nocapture` — 30s × 100 eps × 10 subs; no `OverflowMarker`.
- `mika dev tasks-stream --url http://localhost:8080` against a running mika-spirit: create/complete a task manually and observe frames on stdout.

---

## Test Plan

**Unit tests (in `crates/mika-agent/src/async_db.rs::tests`, or a dedicated `task_engine_event_stream_tests.rs` module):**

1. `test_task_created_frame_emitted_on_create` — attach channel, subscribe, call `create_task`, assert `TaskCreated` frame received with correct fields.
2. `test_task_claimed_frame_emitted_on_successful_claim` — attach channel, subscribe, create task, claim, assert `TaskClaimed`.
3. `test_task_claimed_frame_not_emitted_when_claim_returns_false` — create + cancel, then attempt claim (returns Ok(false)), assert no frame.
4. `test_task_completed_frame_emitted_with_truncated_preview` — attach channel, subscribe, complete task with long result, assert preview <= 500 chars + ellipsis.
5. `test_task_failed_frame_emitted_with_truncated_preview` — mirror of #4 for failures.
6. `test_task_delivered_frame_emitted_on_mark_delivered` — set up completed callback task, `mark_task_delivered`, assert `TaskDelivered`.
7. `test_task_cancelled_frame_emitted_from_cancel_task` — cancel_task returns Ok(true), assert `TaskCancelled`.
8. `test_task_cancelled_frame_emitted_from_update_task_status` — `update_task_status(id, "cancelled")`, assert frame.
9. `test_no_frame_when_channel_unset` — construct AsyncDatabase without attaching channel, run full lifecycle, DB writes succeed, no panic.
10. `test_no_frame_when_zero_subscribers` — attach channel, no subscribers, run full lifecycle, DB writes succeed (`broadcast_frame` returns false, silent).

**Integration test (in `crates/mika-agent/tests/task_event_stream_integration.rs`):**

11. `test_full_lifecycle_create_claim_complete_deliver_emits_four_frames_in_order` — create → claim → complete → mark_delivered; assert exact 4-frame sequence.
12. `test_create_and_cancel_emits_two_frames_in_order` — create → cancel; assert 2 frames.

**Soak (behind `#[ignore]` + `MIKA_TASK_STREAM_SOAK` env, non-CI):**

13. `test_no_overflow_marker_under_matched_drain_rate_10_subscribers_100_eps_30s` — 10 subs each drain in dedicated tasks; producer emits 100 eps × 30s; assert no subscriber received `OverflowMarker`.

---

## Wiring plan (concrete file edits)

**Edit 1 — `crates/mika-agent/src/async_db.rs`:**
- Import `crate::server::tasks_stream::{TaskEventFrame, TaskEventsChannel}` at the module top.
- Add `use std::sync::OnceLock;`.
- In `struct AsyncDatabaseInner`: add `task_events_channel: OnceLock<Arc<TaskEventsChannel>>`.
- In `AsyncDatabase::new_with_agent`: initialise `task_events_channel: OnceLock::new()`.
- New method `pub fn set_task_events_channel(&self, channel: Arc<TaskEventsChannel>)`: `let _ = self.inner.task_events_channel.set(channel);` (ignore already-set error — idempotent-safe attach).
- New method `fn emit_task_event(&self, frame: TaskEventFrame)`: `if let Some(ch) = self.inner.task_events_channel.get() { let _ = ch.broadcast_frame(frame); }`.
- In `create_task(task: NewTask) -> Result<String>`: capture `trigger_type`, `action_type`, `label`, `parent_task_id` clones before move; after `Ok(id)`, emit `TaskEventFrame::TaskCreated { task_id: id.clone(), agent_id, kind, action_type, label, parent_task_id, created_at }`. Return `Ok(id)` unchanged.
- In `claim_and_fire_task`: after `Ok(true)` emit `TaskClaimed`. `Ok(false)` → no emit.
- In `update_task_completed`: after `Ok(true)` emit `TaskCompleted::completed(...)` (via the helper — auto-truncates).
- In `update_task_failed`: after `Ok(true)` emit `TaskFailed::failed(...)`.
- In `promote_task_completed`: after `Ok(true)` emit `TaskCompleted` (source `"retry_promotion"` — pass reason string as result preview).
- In `mark_task_delivered`: after `Ok(true)` emit `TaskDelivered`.
- In `cancel_task`: after `Ok(true)` emit `TaskCancelled { reason: Some("cancelled_via_cancel_task".into()), … }`.
- In `update_task_status`: when `status == "cancelled"` and DB call returns `Ok(())`, emit `TaskCancelled { reason: Some("cancelled_via_update_task_status".into()), … }`. Note: this method's return type is `Result<()>`, no transition-happened bool — the frame fires on any successful call with `"cancelled"`.
- In `update_manual_task_status`: when `new_status == "cancelled"` and DB call returns `Ok(Some(_))`, emit `TaskCancelled { reason: Some("cancelled_via_update_manual_task_status".into()), … }`.

**Edit 2 — `crates/mika-agent/src/server/mod.rs`:**
- Hoist `let task_events_channel = Arc::new(tasks_stream::TaskEventsChannel::new());` to the top of `run_server`, before the agent-init loop.
- Store the same handle into `AppState.task_events_channel`.
- Immediately after each `init_agent` returns an `AgentState`, call `agent_state.db.set_task_events_channel(task_events_channel.clone())`.

**Edit 3 — `crates/mika-agent/src/server/state.rs::resolve_agent`:**
- After `init_agent` returns `Ok(agent_state)` in the lazy-resolve path, call `agent_state.db.set_task_events_channel(self.task_events_channel.clone())` so lazily-resolved agents also emit.

**Edit 4 — `crates/mika-cli/`:**
- Add `mika dev tasks-stream` subcommand (see the mika-cli CLAUDE.md for the dev/hidden namespace pattern) OR fall back to `mika tasks stream` if `dev` namespace doesn't exist. Command opens `GET {MIKA_SPIRIT_URL}/api/v1/dashboard/tasks/stream` with the internal token, pipes SSE `data:` payloads to stdout one line per frame. Uses `reqwest` streaming + `bytes` LineReader pattern.

**Edit 5 — `crates/mika-agent/tests/` or crate-internal `#[cfg(test)] mod`:**
- Integration test file as described in Test Plan.

---

## Risks and mitigations

**R1. Emission race with the DB thread hop.**
The `AsyncDatabase::with_db` pattern hops closures to a dedicated OS thread. `emit_task_event` runs on the calling task's thread AFTER the closure returns. This means the frame is emitted after the DB commit is observable to the calling task, but may be observed by other agent handles before this handle sees `Ok`. That is acceptable: the wire contract is "emit after the DB write commits," and any concurrent reader of the DB will see the committed row before or after the frame — never a frame without a committed row.

**R2. Frame flood on startup recovery.**
`startup_recovery` calls `update_task_status(id, task_status::FAILED)` on orphaned `in_progress` tasks. This is not "in our transition catalog" (the catalog is `update_task_failed`, not `update_task_status` with `"failed"`). No frame will fire from that path — which is arguably correct (startup recovery is not a live transition an observer needs to see) but should be noted. If a follow-up wants startup-recovery visibility, that's a separate ticket.

**R3. `update_task_status` frame ambiguity for non-cancel statuses.**
`update_task_status` is a low-level method; other callers use it with statuses like `"in_progress"`, `"pending"`, `"failed"`. We only emit `TaskCancelled` on `"cancelled"`. Emitting other transitions here would double-fire with the dedicated methods (e.g., `claim_and_fire_task` already emits `TaskClaimed`). The narrow "cancelled only" branch is correct — the dedicated wrappers are the canonical emission sites; `update_task_status` is a leak-through path we cover only for cancel.

**R4. `promote_task_completed` semantic: was the earlier `TaskFailed` frame incorrect?**
No. The earlier `TaskFailed` accurately captured the state at the time. The subsequent `TaskCompleted` captures the promotion. Consumers who care about the final state can ignore intermediate `TaskFailed` when a `TaskCompleted` for the same task_id lands later; the wire faithfully records the sequence.

**R5. Test call sites constructing `AsyncDatabase` are unaffected.**
~40 test call sites use `AsyncDatabase::new_with_agent(db, "…")` without the channel. `OnceLock::get()` returns `None`, `emit_task_event` no-ops, DB writes succeed. Zero test churn.

**R6. Multi-tenant per-agent scoping deferred.**
Per the ticket body's "Plumbing" and "DashMap upgrade path" sections, per-agent scoping is a v2 concern. Frames carry `agent_id`; consumers filter client-side. This ticket does not thread agent_id through the DB layer.

---

## Out of scope

- TUI status pane integration (mika#1727).
- Sibling permission-decision SSE stream emission (mika#1741 — separate ticket).
- Multi-tenant per-agent broadcast scoping (documented in ticket body §DashMap upgrade path; deferred).
- `?agent_id=X` query filter on the SSE handler (cheap upgrade, deferred to a follow-up when a consumer needs it).
- Startup-recovery emissions (see Risk R2).
- `mark_tasks_expired` transitions to the `expired` status — the wire enum has no `TaskExpired` variant. Adding one is an additive follow-up; the v1 wire stays silent on this transition.
- `cancel_recurring_task_by_label` per-row cancel frames (see KTD11 — signature widening deferred).
- Cascade cancellations inside `Database::cancel_task` (parent-cancel flips child callbacks to `cancelled` inside the same SQL transaction — the wrapper only observes the parent transition).

---

## References

- `crates/mika-agent/src/server/tasks_stream.rs` — the wire (enum, channel, handler) from mika#1732.
- `crates/mika-agent/src/async_db.rs` — the wrapper methods this plan modifies.
- `crates/mika-agent/src/task_engine/dispatcher.rs`, `crates/mika-agent/src/task_engine/engine.rs` — indirect callers of the wrapper methods.
- `docs/plans/2026-07-10-004-feat-1732-task-event-sse-plan.md` §Out of scope — the deferral that this plan resolves.
- `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md` — the frame catalog.
- `docs/architecture.md` §14.2 — SSE Frame Catalog.
- mika-arch grooming session `4ac4247a-0d38-4ac2-a8fe-6196fe31bf9f` — the 14-day-or-P1 architect endorsement.
