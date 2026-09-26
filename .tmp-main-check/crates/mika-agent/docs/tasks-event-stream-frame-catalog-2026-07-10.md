# Task-event live stream frame catalog

**Ticket:** mika#1732 (sub-B of mika#1727 TUI thin-client)
**Date:** 2026-07-10
**Wire ships in:** PR-to-be from branch `feat/1732/new-sse-endpoint-task-event-live-stream`

## Purpose

Enumerate every SSE frame type the `GET /api/v1/dashboard/tasks/stream` handler broadcasts (once emission is wired). Consumers (TUI status pane from mika#1727, dashboard task-flow subscribers, future sub-tickets) should treat this as the single authoritative reference for the wire shape.

## Transport

- Handler: `handle_tasks_events_stream` at `crates/mika-agent/src/server/tasks_stream.rs`.
- Route: `GET /api/v1/dashboard/tasks/stream` — registered in `dashboard_routes` at `crates/mika-agent/src/server/mod.rs` alongside `/dashboard/permissions/stream`.
- Auth: `require_dashboard_or_internal_token` middleware — accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser).
- Frame encoding: plain `axum::response::sse::Event::default().data(json_string)`. No `event:` or `id:` header on the SSE envelope — the discriminator lives inside the JSON body via `#[serde(tag = "event")]`. `KeepAlive::default()` heartbeat.
- Channel: per-process single `Arc<TaskEventsChannel>` on `AppState.task_events_channel`. Wraps a `tokio::sync::broadcast::Sender<TaskEventFrame>` with capacity `CHANNEL_CAP = 256`. Rationale: task-transition traffic can burst higher than permissions traffic (dispatch cascade, callback delivery flurry); 2× the permissions cap of 128.
- Slow-consumer discipline: `BroadcastStreamRecvError::Lagged(n)` is translated to a `TaskEventFrame::OverflowMarker { dropped_count: n }` frame on the wire. Drop-oldest is Tokio's default. Emission is fire-and-forget — `broadcast_frame().is_ok()` returning `false` (zero subscribers) is informational, not an error.

## `TaskEventFrame` variants

Six event variants + one overflow marker + one forward-compat catch-all.

### 1. `TaskCreated` — `"event": "task_created"`

Fired on `db.create_task()` after a successful INSERT.

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | The new task ID |
| `agentId` | `String` | Owning agent (consumers may filter client-side) |
| `kind` | `String` | Task trigger_type: `manual` \| `callback` \| `recurring` \| `a2a` |
| `actionType` | `String` | E.g. `resume_agent`, `send_message`, `none` |
| `label` | `Option<String>` | Task label if set |
| `parentTaskId` | `Option<String>` | Parent task if a child task |
| `createdAt` | `String` | ISO 8601 UTC |

### 2. `TaskClaimed` — `"event": "task_claimed"`

Fired when a task transitions to `in_progress` (dispatch fired).

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `agentId` | `String` | |
| `claimedAt` | `String` | ISO 8601 UTC |

### 3. `TaskCompleted` — `"event": "task_completed"`

Fired on `db.update_task_completed()`.

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `agentId` | `String` | |
| `completedAt` | `String` | ISO 8601 UTC |
| `resultPreview` | `Option<String>` | 500-char UTF-8-safe truncated result. Full body via `GET /api/v1/tasks/{taskId}` |

### 4. `TaskFailed` — `"event": "task_failed"`

Fired on `db.update_task_failed()`.

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `agentId` | `String` | |
| `failedAt` | `String` | ISO 8601 UTC |
| `errorPreview` | `Option<String>` | 500-char UTF-8-safe truncated error summary |

### 5. `TaskDelivered` — `"event": "task_delivered"`

Fired on `db.mark_task_delivered()` when a completed / failed callback is consumed by the resume dispatcher.

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `agentId` | `String` | |
| `deliveredAt` | `String` | ISO 8601 UTC |

### 6. `TaskCancelled` — `"event": "task_cancelled"`

Fired on `db.update_task_status(id, "cancelled")` from HTTP handlers or engine-driven cancels.

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `agentId` | `String` | |
| `cancelledAt` | `String` | ISO 8601 UTC |
| `reason` | `Option<String>` | Cancellation reason if set |

### 7. `OverflowMarker` — `"event": "overflow_marker"`

Emitted by the SSE handler (not a task-engine site) when `BroadcastStreamRecvError::Lagged(n)` fires. Signals to the consumer that at least one frame was dropped due to slow-consumer backpressure.

| Field | Type | Notes |
|---|---|---|
| `droppedCount` | `u64` | Count of dropped frames since the last successful recv |

### 8. `Unknown` — `#[serde(other)]` catch-all

Forward-compat variant. Deserializes any `event` value not matched by the above variants. Consumers should match `Unknown` and fall through with a log-and-ignore branch.

## Sibling divergence — do not conflate

Three sibling SSE surfaces now coexist:

| Surface | Discriminator | Scope | Route pattern | Crate |
|---|---|---|---|---|
| `mika_a2a::streaming::StreamEvent` (mika#1731) | `tag = "kind"` | Per-task | JSON-RPC-inline SSE | `mika-a2a` |
| `PermissionStreamFrame` (mika#1741) | `tag = "event"` | Per-process | `GET /dashboard/permissions/stream` | `mika-agent` server |
| `TaskEventFrame` (mika#1732, this file) | `tag = "event"` | Per-process | `GET /dashboard/tasks/stream` | `mika-agent` server |

The A2A vs Dashboard divergence is intentional and load-bearing: A2A is JSON-RPC transport with task-scoped sessions; Dashboard is HTTP SSE with global tenant-scoped streams. `TaskEventFrame` follows `PermissionStreamFrame` because it lives in the same Dashboard family, not because the discriminator was arbitrary. Do NOT retrofit `StreamEvent` to `tag = "event"` — that would break existing A2A consumers.

## Emission sites (for the follow-up ticket)

The wire ships in this PR. Emission from task_engine transition sites is a follow-up (see plan §Out of scope). Top-level catalog for the emitter:

- **`db.create_task()` → `TaskCreated`** — every call site. Consider emitting from the `AsyncDatabase::create_task_async` wrapper for centralization.
- **`db.claim_and_fire_task()` → `TaskClaimed`** — `dispatcher.rs` at each dispatch site.
- **`db.update_task_completed()` → `TaskCompleted`** — `dispatcher.rs:1702, 1752, 636, 647`; `engine.rs:837, 871`.
- **`db.update_task_failed()` → `TaskFailed`** — `dispatcher.rs:1368, 1403`; `engine.rs:661, 744, 482`.
- **`db.mark_task_delivered()` → `TaskDelivered`** — `dispatcher.rs:481`; `engine.rs:417`.
- **`db.update_task_status(id, "cancelled")` → `TaskCancelled`** — `handle_task_cancel` in `server/handlers.rs`, `cancel_task_and_kill`, `process_kill.rs`.

Emit AFTER the successful DB write. Frame captures the AFTER state — the caller reads the same fields it wrote (timestamps, agent_id, etc.) to populate.

## Truncation helper

`truncate_preview(s: &str) -> String` at `crates/mika-agent/src/server/tasks_stream.rs`. UTF-8-safe: iterates codepoints, takes the first 500, appends `…` if any were dropped. Guards against mika#764 byte-slice-lint class.

## Related docs

- `docs/architecture.md` §14.2 SSE Frame Catalog — cross-references all three sibling enums.
- `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` — mika#1731 sibling verification note.
- `docs/plans/2026-07-10-004-feat-1732-task-event-sse-plan.md` — this ticket's plan.
