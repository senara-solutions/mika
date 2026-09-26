---
ticket: mika#1732
branch: feat/1732/new-sse-endpoint-task-event-live-stream
type: feat
scope: crates/mika-agent
grooming: /mika-groom-ticket
parent: mika#1727 (Phase 1 audit sub-ticket B)
---

# Plan — mika#1732 task-event SSE stream for TUI status pane

## Problem

Sub-issue of mika#1727 (TUI as thin HTTP client of mika-spirit). The TUI's status pane wants live task-transition events (created / claimed / completed / failed / delivered) as they happen. Today the closest surface is `GET /api/v1/tasks` (snapshot list at `crates/mika-agent/src/server/dashboard.rs:639`); a thin-client TUI would have to poll it, which wastes cycles and produces stale rendering between polls. This ticket adds an SSE endpoint that pushes task events as they happen.

## Verification result

**Verified — new SSE surface is required.** The current mika-spirit HTTP surface has:

- `GET /api/v1/tasks` (`crates/mika-agent/src/server/mod.rs:132`, handler at `dashboard.rs:639-678`) — paginated snapshot returning `PaginatedResponse<TaskResponse>`. Query filters (status, trigger_type, action_type, agent_id, team_run_id, source, from, to). Single-shot; no subscription.
- `GET /api/v1/tasks/{task_id}` — detail.
- No streaming route for task transitions exists. Zero broadcast channel for task events.
- **Reusable sibling primitive:** `PermissionsChannel` at `crates/mika-agent/src/server/permissions_stream.rs` (shipped in mika#1741). Per-process `broadcast::Sender<PermissionStreamFrame>` on `AppState.permissions_channel` (`state.rs:107`), capacity 128, `#[serde(tag = "event", rename_all = "snake_case")]`, explicit `OverflowMarker { dropped_count }` frame that translates `BroadcastStreamRecvError::Lagged` into the wire (`permissions_stream.rs:199-225`). Route registered at `server/mod.rs:268-275` with `require_dashboard_or_internal_token` middleware (`server/auth.rs:48`) via the `dashboard_routes` block. This is exactly the shape #1732 needs.

Task lifecycle transition sites are extensive — every `db.update_task_completed`, `db.update_task_failed`, `db.mark_task_delivered`, `db.claim_and_fire_task`, `db.update_task_status`, `db.create_task` is a potential emit point. Full list documented in the pre-groom investigation (agent scratchpad; §Emission sites below has the top-level summary).

## Coordination with mika#1731

mika#1731 (PR#1756, awaiting review) is **strictly additive on mika-a2a** — new variants on `mika_a2a::streaming::StreamEvent` in a different crate. This ticket is **strictly additive on mika-agent server** — new module `server/tasks_stream.rs`, new field on `AppState`, new route. **No coordination required.** Both PRs land independently.

## Scope

### In scope for v1 (this PR — the WIRE)

Same wire-first split samidarko-claude endorsed on mika#1731 (PR#1756) and mika#1741 (PR#1741). Ship the SSE surface + channel primitive + frame catalog + unit tests. Defer the emission-from-transition-sites plumbing to a follow-up ticket that lands when a consumer (mika#1727 TUI) is ready.

**AC1 — New module `crates/mika-agent/src/server/tasks_stream.rs`.** Mirrors `permissions_stream.rs` in shape:

- `TaskEventFrame` enum with `#[serde(tag = "event", rename_all = "snake_case")]` — same convention as `PermissionStreamFrame` (server-crate dashboard SSE family; NOT the mika-a2a `StreamEvent` `tag = "kind"` convention which is per-task JSON-RPC-inline SSE). This is a deliberate choice — the ticket is a sibling of `permissions_stream`, not of `message/stream`.
- Six variants + one overflow + one catch-all (details in §Implementation guardrails).
- `TaskEventsChannel` struct wrapping `broadcast::Sender<TaskEventFrame>`, capacity `TASK_EVENTS_CHANNEL_CAP = 256`. Rationale: task-transition traffic can burst (dispatch cascade, callback delivery flurry) higher than permissions traffic; double the permissions cap of 128.
- `broadcast_frame(&self, frame: TaskEventFrame) -> bool` — non-blocking, `send().is_ok()`. Zero subscribers is informational.
- `handle_tasks_events_stream(State<AppState>)` — SSE handler mirroring `handle_permissions_stream` at `permissions_stream.rs:180-233`. Uses `BroadcastStream::new(rx).filter_map(...)` to translate `Lagged(n)` into a `TaskEventFrame::OverflowMarker { dropped_count: n }` on the wire.
- `receiver_count()` accessor gated `#[cfg(test)]`.

**AC2 — Route registration.** `GET /api/v1/dashboard/tasks/stream` (or `/api/v1/tasks/stream` — the exact path is a small grooming call; recommend `/api/v1/dashboard/tasks/stream` to mirror `/dashboard/permissions/stream` for discoverability). Wired into `dashboard_routes` at `crates/mika-agent/src/server/mod.rs` alongside the existing `/dashboard/permissions/stream` and `/dashboard/permissions/{request_id}/decide` routes at `server/mod.rs:268-275`. Auth via the shared `dashboard_routes.route_layer(...)` at `server/mod.rs:276-279` — `require_dashboard_or_internal_token`. **No new auth class; no per-tenant scope changes.**

**AC3 — `AppState.task_events_channel: Arc<TaskEventsChannel>` field.** Constructed identically to `permissions_channel` in `test_state()` at `server/mod.rs:1602` and in production init at `server/mod.rs:1314`. Single per-process instance. Frames carry `agent_id` for consumer-side filtering — no per-agent DashMap. Justification: single-tenant deploy shape (mika#1727 audit: "single-user boxes run it as a local process on 127.0.0.1:8081") + parity with `PermissionsChannel` which is also per-process.

**AC4 — Slow-consumer discipline.** Handled by `broadcast::channel`'s built-in `Lagged` + an explicit `TaskEventFrame::OverflowMarker { dropped_count: u64 }` frame that surfaces on the wire when the client falls behind. `permissions_stream.rs:199-225` is the reference. Drop-oldest is Tokio's default for `broadcast::channel`; no additional buffering layer. Emission is fire-and-forget: `broadcast_frame().is_ok()` returning `false` (zero subscribers) does not fail the caller.

**AC5 — Frame enum shape.**

```rust
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEventFrame {
    /// A new task row was written to `tasks`. Fired on `db.create_task()`.
    TaskCreated {
        task_id: String,
        agent_id: String,
        kind: String,          // "manual" | "callback" | "recurring" | "a2a"
        action_type: String,   // e.g. "resume_agent", "none", "send_message"
        label: Option<String>,
        parent_task_id: Option<String>,
        created_at: String,
    },
    /// A task transitioned to `in_progress` (dispatch fired).
    TaskClaimed {
        task_id: String,
        agent_id: String,
        claimed_at: String,
    },
    /// A task transitioned to `completed`.
    TaskCompleted {
        task_id: String,
        agent_id: String,
        completed_at: String,
        result_preview: Option<String>,   // truncated at 500 chars
    },
    /// A task transitioned to `failed`.
    TaskFailed {
        task_id: String,
        agent_id: String,
        failed_at: String,
        error_preview: Option<String>,    // truncated at 500 chars
    },
    /// A completed/failed callback was consumed by the resume dispatcher.
    TaskDelivered {
        task_id: String,
        agent_id: String,
        delivered_at: String,
    },
    /// A task transitioned to `cancelled` (operator or engine-driven).
    TaskCancelled {
        task_id: String,
        agent_id: String,
        cancelled_at: String,
        reason: Option<String>,
    },
    /// Emitted by the SSE handler when `BroadcastStream` reports `Lagged(n)` —
    /// the client fell behind and n frames were dropped.
    OverflowMarker { dropped_count: u64 },
    /// Forward-compat catch-all — future variants ship additively.
    #[serde(other)]
    Unknown,
}
```

Same forward-compat catch-all pattern as mika#1731 (`StreamEvent::Unknown`). Kebab-case `event` values via `rename_all = "snake_case"` — matches `PermissionStreamFrame` convention.

**AC6 — Truncation policy.** `result_preview` and `error_preview` are UTF-8-safe truncated at 500 chars via a reused or inlined `truncate_utf8_safe` helper. If mika#1731 lands first, reuse `mika_a2a::streaming::truncate_tool_summary`. If this PR lands first, inline a `truncate_summary` helper in `tasks_stream.rs` — same shape (codepoint iterator + `…` suffix on overflow, guards against mika#764 byte-slice lint).

**AC7 — Unit tests.** Match `permissions_stream.rs:274-374` exactly:
- Frame serde round-trip for every emit variant (six of them + OverflowMarker).
- `event` tag value stability test — asserts the wire strings (`"task_created"`, `"task_claimed"`, `"task_completed"`, `"task_failed"`, `"task_delivered"`, `"task_cancelled"`, `"overflow_marker"`) to catch silent breakage.
- `Unknown` catch-all deserialization test — an unknown `event` value lands in `Unknown` without panic.
- `broadcast_frame_reaches_subscriber` — subscribe → emit → recv, mirrors `permissions_stream.rs:354`.
- `broadcast_frame_zero_subscribers_no_error` — `broadcast_frame` returns cleanly with no receivers.
- Optional: `overflow_marker_fires_on_lag` — send `TASK_EVENTS_CHANNEL_CAP + 10` frames without draining, verify `OverflowMarker` surfaces. This is testable at the `BroadcastStream` layer without a full router.

**AC8 — Docs.**
- `docs/architecture.md` §14.2 SSE Frame Catalog (already established by mika#1731; if #1731 hasn't merged yet, this PR either extends it if #1731 lands first or introduces the §14.2 subsection heading itself and references the mika-a2a StreamEvent enum without duplication). Add `TaskEventFrame` row + cross-reference to `PermissionStreamFrame` and `StreamEvent` — three sibling SSE surfaces with divergent scopes/tags but shared "additive + catch-all" discipline.
- New verification note at `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md` — mirrors the mika#1731 catalog note. Emit-site table pointing to §Emission sites (below) for post-emission-PR wiring.

**AC9 — Build + lint clean.**
- `cargo build -p mika-agent`
- `cargo test -p mika-agent --lib tasks_stream`
- `cargo clippy --workspace --all-targets -- -D warnings`

### Out of scope for v1 (deferred to follow-up mika#new)

**AC5 of the ticket (emission wiring) is scope-reduced from this PR.** The emission sites are extensive — every DB-side transition needs a corresponding `broadcast_frame` call. That plumbing requires:

- Threading `Arc<TaskEventsChannel>` into `TaskDispatcher` (`task_engine/dispatcher.rs`) and `TaskEngine` (`task_engine/engine.rs`) at construction time (mika#1732 v1 does NOT touch these files — the `AsyncDatabase` API is agnostic to the channel).
- Or emitting from callers immediately after successful DB writes. Callers include `dispatcher.rs:481, 636, 647, 1159, 1194, 1603, 1702, 1752`, `engine.rs:661, 744, 837, 871, 384, 417, 453, 482`, plus HTTP handlers `server/handlers.rs::handle_task_complete`, `handle_task_cancel`, plus `task_engine/process_kill.rs:404, 464`, plus every `db.create_task(NewTask{...})` site (widespread).
- Deciding on read-then-write vs read-post-write ordering (frame captures the AFTER state; the caller must read the new state to populate the frame fields — or the frame is written before the SQL commit and rolled back on failure — a real design question, not a mechanical rewrite).
- Integration test for the full lifecycle (10 concurrent subscribers + 100 events/sec, ticket AC5's load target).

That's a substantial diff on top of the wire. Split to follow-up ticket **mika#new** (to be filed alongside this PR — mirrors mika#1757 for #1731). Reactivate when mika#1727 (TUI) is ready to consume the frames.

**Also out of scope for v1:**
- **TUI rendering** — mika#1727 closing PR.
- **Cursor replay** — the ticket's discipline lineage (D5 / cm#99) mentions replay-on-reconnect. Not part of v1; if the TUI needs it, a separate ticket adds a per-frame monotonic ID column + replay handler. Same pattern as mika#1741 which shipped without replay.
- **Cross-tenant subscription policies** — single-tenant Phase 1 shape (ticket §Not in scope).
- **Historical query** — ticket §Not in scope; `GET /api/v1/tasks` snapshot remains.
- **Heartbeat frame** — ticket lists as "optional; for phantom-task detection". Deferred to the emission follow-up when we know if the task_engine actually needs it separately from `TaskClaimed` / periodic `TaskEvent::updated_at`.

## Implementation guardrails

### File and function targets

| Change | File | Location |
|---|---|---|
| New module — enum, channel primitive, handler, tests | `crates/mika-agent/src/server/tasks_stream.rs` | New file |
| Module registration | `crates/mika-agent/src/server/mod.rs` | Near `pub mod permissions_stream;` |
| Route registration | `crates/mika-agent/src/server/mod.rs` | In `dashboard_routes` block near `/dashboard/permissions/stream` at line 268 |
| `AppState.task_events_channel` field | `crates/mika-agent/src/server/state.rs` | Near `permissions_channel` at line 107 |
| `AppState` construction | `crates/mika-agent/src/server/mod.rs` | Prod init (~line 1314), test-state factory (~line 1602) |
| Architecture doc §14.2 | `docs/architecture.md` + doc-synced copy | Add `TaskEventFrame` row |
| Verification note | `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md` | New file |

### Emission sites (for the follow-up ticket)

Top-level catalog for the emission-plumbing PR. Every site emits AFTER the successful DB write.

- **`db.create_task()` → `TaskCreated`** — every call site. High-cardinality (many creation paths); consider emitting from `AsyncDatabase::create_task_async` wrapper rather than callers if that's the shape task_engine settles on.
- **`db.claim_and_fire_task()` → `TaskClaimed`** — `dispatcher.rs` at each dispatch site.
- **`db.update_task_completed()` → `TaskCompleted`** — `dispatcher.rs:1702, 1752, 636, 647`, `engine.rs:837, 871`.
- **`db.update_task_failed()` → `TaskFailed`** — `dispatcher.rs:1368, 1403`, `engine.rs:661, 744, 482`.
- **`db.mark_task_delivered()` → `TaskDelivered`** — `dispatcher.rs:481`, `engine.rs:417`.
- **`db.update_task_status(id, "cancelled")` → `TaskCancelled`** — `handle_task_cancel` in `server/handlers.rs`, `cancel_task_and_kill`, `process_kill.rs`.

### Backwards compatibility

- No existing frame renamed or removed (no existing frames on this route).
- No existing route touched.
- No auth-class change.
- No schema migration.
- New `AppState` field; every construction site of `AppState` must include `task_events_channel: Arc::new(TaskEventsChannel::new())`. Two sites (prod + test-state).

### Threading + Send bounds

`TaskEventsChannel` must be `Send + Sync + Clone` (via `Arc<...>`). `broadcast::Sender<T>` is `Clone + Send + Sync` when `T: Send`. `TaskEventFrame` derives `Debug, Clone, Serialize, Deserialize` — the derived `Clone` is trivial (all fields are `String`/`Option<String>`/`u64`), no manual impl needed.

### Documentation update — §14.2 SSE Frame Catalog

If #1731 lands first, this PR extends the existing §14.2 table with a `TaskEventFrame` row + cross-reference to `PermissionStreamFrame` and `StreamEvent` — one sentence noting the four divergence axes (discriminator key, correlation scope, route, crate location).

If this PR lands first, introduce §14.2 heading standalone. #1731's PR (or a follow-up) will merge its content in when it lands.

## Acceptance criteria

**AC1.** New module `crates/mika-agent/src/server/tasks_stream.rs` defines `TaskEventFrame` enum with the six event variants, `OverflowMarker`, and `Unknown` catch-all. `#[serde(tag = "event", rename_all = "snake_case")]`.

**AC2.** `TaskEventsChannel` struct wraps `broadcast::Sender<TaskEventFrame>` with capacity 256, exposes `broadcast_frame(&self, frame: TaskEventFrame) -> bool` and `subscribe(&self) -> broadcast::Receiver<TaskEventFrame>`. Test-only `receiver_count()` accessor.

**AC3.** `handle_tasks_events_stream(State<AppState>) -> Sse<...>` handler translates `BroadcastStreamRecvError::Lagged(n)` into `TaskEventFrame::OverflowMarker { dropped_count: n }` on the wire. `KeepAlive::default()` heartbeat.

**AC4.** Route `GET /api/v1/dashboard/tasks/stream` is registered inside `dashboard_routes` at `server/mod.rs`, guarded by `require_dashboard_or_internal_token` middleware.

**AC5.** `AppState.task_events_channel: Arc<TaskEventsChannel>` field is populated in both production init and `test_state()`.

**AC6.** Truncation of `result_preview` / `error_preview` at 500 chars is UTF-8-safe (codepoint-boundary + U+2026 ellipsis on overflow). No naive byte slicing (guards against mika#764 lint script).

**AC7.** Unit tests inside `tasks_stream.rs` cover: (a) serde round-trip for every variant, (b) `event` tag string stability for each variant, (c) `Unknown` catch-all deserializes without panic on unknown `event` value, (d) `broadcast_frame_reaches_subscriber` positive path, (e) zero-subscribers fires without error.

**AC8.** `docs/architecture.md` §14.2 SSE Frame Catalog gets a `TaskEventFrame` row + cross-reference to `PermissionStreamFrame` (mika#1741) and `StreamEvent` (mika#1731). New verification note at `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md`. Doc-synced via `scripts/sync-agent-docs.sh`.

**AC9.** `cargo build -p mika-agent`, `cargo test -p mika-agent --lib tasks_stream`, and `cargo clippy --workspace --all-targets -- -D warnings` all pass.

## Verification steps (post-implementation)

1. `cargo test -p mika-agent --lib tasks_stream` — new unit tests green.
2. `cargo clippy --workspace --all-targets -- -D warnings` — clean.
3. Manual (post-emission-follow-up only): local mika-spirit + `curl -N -H "Authorization: Bearer $MIKA_INTERNAL_TOKEN" http://localhost:8081/api/v1/dashboard/tasks/stream`, observe SSE stream. Any operator action creating/completing a task should produce a frame. This is documented for the emission follow-up PR, not this one.

## Rollout

- Merge to `main` → next `make deploy` picks it up. No cluster ops.
- No consumer yet; the wire is dormant. When the emission follow-up lands, dashboard subscribers begin receiving frames. When mika#1727 (TUI) migrates, that's the actual UX unlock.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Broadcast channel capacity 256 chosen by gut. | Channel-level tests validate `OverflowMarker` behavior; if production traffic overflows sustainedly, capacity is a one-const-edit change. Track in the emission-follow-up if observed. |
| Extra `AppState` field must be added everywhere `AppState` is constructed. | Only 2 sites: prod init at `server/mod.rs:1314`, `test_state()` at `server/mod.rs:1602`. Compiler-enforced. |
| Emission from every task-transition site is a large diff. | Deferred to follow-up ticket per samidarko-endorsed wire-first pattern. This PR ships only the surface. |
| Frame captures BEFORE vs AFTER state at emission. | Emit AFTER successful DB write. Frame constructor reads the same fields the caller wrote (timestamps, agent_id, etc.). Documented in the follow-up ticket. |
| Downstream client-side filtering by agent_id is a scan. | Acceptable for single-tenant deploy. If multi-tenant becomes real, add a `?agent_id=X` query filter at the handler; frames still pass the whole enum through the channel — filter at consumption. Follow-up. |
| Doc section §14.2 SSE Frame Catalog collision with mika#1731. | mika#1731 introduces §14.2 in PR#1756. If PR#1756 has not merged by review-time, this PR extends the same table; if it has, ditto. Additive. If both PRs miss each other, one merges its section and the other rebase-adds its row — no risk of loss. |

## Files changed (expected)

- `crates/mika-agent/src/server/tasks_stream.rs` — new module. ~350 lines (enum + channel + handler + tests).
- `crates/mika-agent/src/server/mod.rs` — module registration + route + 2 AppState construction sites. ~10 lines added.
- `crates/mika-agent/src/server/state.rs` — 1 new field. ~3 lines.
- `crates/mika-agent/docs/tasks-event-stream-frame-catalog-2026-07-10.md` — new verification note. ~100 lines.
- `docs/architecture.md` + `crates/mika-agent/docs/architecture.md` (via doc-sync) — §14.2 addition. ~15 lines.

Estimated diff: ~480 net lines added.

## Grooming history

- 2026-07-10 — `/ce:plan` draft (with pre-groom verification pass).
- 2026-07-10 — `mika-arch` first-pass review (session `4ac4247a-0d38-4ac2-a8fe-6196fe31bf9f`): **Disposition: READY**. All three uncertainties confirmed. Two refinements to fold into the follow-up ticket:
  1. Follow-up must document the DashMap upgrade path explicitly. Note that `db.rs`-layer emitters are agent-agnostic, so per-agent scoping requires either threading `agent_id` through the DB layer (wrong layer concern) or keeping a parallel per-process channel for DB-layer events.
  2. Follow-up must carry a **time-bound commitment** — the emission plumbing follow-up should merge within 14 days of this PR or be escalated to P1. Prevents the "permissions_stream has been dead code for months" recurrence. Load-bearing for mika#1727.
- Architect also endorsed documenting the current three-axis divergence between A2A and Dashboard SSE families as a first-class architectural invariant in §14.2, with an additional row noting the family membership (A2A vs Dashboard) as an implicit fourth axis (crate location).
