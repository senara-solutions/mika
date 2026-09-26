# A2A `message/stream` SSE frame catalog

**Ticket:** mika#1731 (sub-A of mika#1727 TUI thin-client)
**Date:** 2026-07-10
**Verification pass on:** commit `71bf5ee7` (branch `main` prior to this ticket)

## Purpose

Enumerate every SSE frame type the A2A `message/stream` handler emits or defines, so downstream consumers (TUI thin-client mika#1727, dashboard subscribers, future sub-tickets) have a single authoritative reference for the wire shape.

## Transport

- Handler: `handle_message_stream` at `crates/mika-agent/src/server/a2a.rs:309-480`.
- Route: JSON-RPC `message/stream` method, response body is Axum `Sse<...>` with `KeepAlive::default()`.
- Frame encoding: plain `Event::default().data(json_string)`. No `event:` or `id:` header on the SSE envelope — the discriminator lives inside the JSON body via `#[serde(tag = "kind")]`.
- Per-task broadcast: `state.a2a_broadcasters: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>`. One `broadcast::Sender<StreamEvent>` per outstanding task (bounded channel, capacity 32). Guarded by `BroadcasterGuard` (RAII drop clears the entry).

## `StreamEvent` variants (enum at `crates/mika-a2a/src/streaming.rs`)

### 1. `StatusUpdate` — `"kind": "status-update"`

The only frame class actually broadcast on `message/stream` today. Emitted at three lifecycle points:

| Emit site | State | `is_final` | Message payload |
|---|---|---|---|
| `a2a.rs:382` | `Working` | `false` | `None` |
| `a2a.rs:427` | `Completed` | `true` | `Some(response_message)` — the terminal assistant text as `Part::Text` |
| `a2a.rs:446` | `Failed` | `true` | `None` |

Fields (`TaskStatusUpdateEvent`):

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | Correlates to the JSON-RPC task |
| `contextId` | `Option<String>` | Optional threading identifier |
| `status.state` | `TaskState` | `Working` \| `Completed` \| `Failed` \| … |
| `status.message` | `Option<Message>` | Only set on `Completed` |
| `status.timestamp` | `Option<String>` | RFC 3339 |
| `final` (JSON) / `is_final` (Rust) | `bool` | `true` on terminal states |
| `metadata` | `Option<HashMap<String, Value>>` | Currently unused |

### 2. `Task` — `"kind": "task"`

Defined at `streaming.rs:11`, but **NEVER emitted by `message/stream`** in production code. Only referenced as a fallback in `handle_tasks_resubscribe` (`a2a.rs:788`) when the broadcaster is already gone.

### 3. `Message` — `"kind": "message"`

Defined at `streaming.rs:13`, but **NEVER emitted anywhere in production**. Only test scaffolding at `streaming.rs:107`.

### 4. `ArtifactUpdate` — `"kind": "artifact-update"`

Defined at `streaming.rs:17`, but **NEVER emitted anywhere in production**. Only test scaffolding at `streaming.rs:71`.

### 5. `ToolCallStart` — `"kind": "tool-call-start"` (mika#1731, this ticket)

Payload (`ToolCallStartEvent`):

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | Same correlation as `StatusUpdate.taskId` |
| `contextId` | `Option<String>` | |
| `step` | `u32` | Matches `ToolCallSummary.step` — the tool step number within the current agent turn |
| `toolName` | `String` | E.g. `"run_gh"`, `"store_fact"`, `"read_agent_file"` |
| `argsSummary` | `String` | Serialized arguments, UTF-8-safe truncated to `TOOL_CALL_SUMMARY_CAP_CHARS` (500) with trailing `…` when over cap |
| `timestamp` | `String` | RFC 3339 UTC |

Emit contract (v1 ships the type; emission plumbing tracked as follow-up):
- Emitted immediately BEFORE `execute_tool()` in `process_tool_calls` (`crates/mika-agent/src/tool_execution/dispatch.rs:43`).
- Fire-and-forget: `broadcast::Sender::send()` errors indicate zero subscribers or lag-drop; log at `debug!` and continue. Never fail the tool call.
- Per-turn dedup replays (`ToolContext` cached `ToolOutput` from mika#582) do NOT emit. Only the physically-dispatched call emits.

### 6. `ToolCallResult` — `"kind": "tool-call-result"` (mika#1731, this ticket)

Payload (`ToolCallResultEvent`):

| Field | Type | Notes |
|---|---|---|
| `taskId` | `String` | |
| `contextId` | `Option<String>` | |
| `step` | `u32` | Pairs with the matching `ToolCallStart.step` |
| `toolName` | `String` | |
| `success` | `bool` | Same discriminator as `ToolCallSummary.success` |
| `nonZeroExit` | `bool` | Same discriminator as `ToolCallSummary.non_zero_exit` (exec-handler heuristic) |
| `outputSummary` | `String` | UTF-8-safe truncated to 500 chars |
| `durationMs` | `u64` | Wall-clock duration of `execute_tool()` |
| `timestamp` | `String` | RFC 3339 UTC |

Same fire-and-forget policy as `ToolCallStart`.

### 7. `Unknown` — `#[serde(other)]` catch-all (mika#1731, this ticket)

Forward-compat variant. Deserializes any `kind` value not matched by the above variants. Consumers should match `Unknown` and fall through with a log-and-ignore branch.

Rationale: mika#1732 (task-event stream), mika#1734 (AskUserQuestion bridge), mika#1736 (session-messages ordered stream), and later sub-tickets each add more variants. Without the catch-all, every wire extension would require a synchronous re-release of every consumer. With the catch-all, producers append variants and consumers upgrade opportunistically.

## Truncation helper

`truncate_tool_summary(s: &str) -> String` at `crates/mika-a2a/src/streaming.rs`. UTF-8-safe: iterates codepoints, takes the first 500, appends `…` if any were dropped. Guards against the mika#764 byte-slice-lint class (naive `&str[..N]` panics on multi-byte codepoints).

## Sibling SSE surface — `PermissionStreamFrame` (mika#1741)

**Not to be conflated.** Lives at `crates/mika-agent/src/server/permissions_stream.rs:54-74`. Three axes of deliberate divergence documented at `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md`:

| Axis | `StreamEvent` (this file) | `PermissionStreamFrame` |
|---|---|---|
| Discriminator key | `#[serde(tag = "kind")]` | `#[serde(tag = "event")]` |
| Correlation scope | Per-task (`a2a_broadcasters: DashMap<TaskId, Sender>`) | Per-process (single global `PermissionsChannel`) |
| Route | JSON-RPC POST body carries the SSE (inline) | Dedicated `GET /api/v1/dashboard/permissions/stream` |

Both enums ship with forward-compat catch-alls. Neither should be extended by variants from the other — the wire shapes are distinct by design.

## Consumer contract

### Existing consumer

- `crates/mika-cli/src/remote_ask.rs` — calls `message/send` (non-streaming). Does NOT parse SSE frames today.
- `crates/mika-a2a/src/client.rs` — `A2aClient::send_message_streaming()` + `parse_sse_stream()` exist but are unused (tracked at `todos/711-complete-p2-dead-code-unused-client-methods.md`).

### Future consumer

- mika#1727 (TUI thin-client) — will migrate `remote_ask.rs` to `message/stream` and render `ToolCallStart` / `ToolCallResult` inline.

## Related docs

- `docs/architecture.md` §14 — A2A protocol architecture (extended by this ticket to include the SSE frame catalog subsection).
- `docs/plans/2026-07-10-003-feat-1731-a2a-tool-call-sse-plan.md` — this ticket's plan.
- `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md` — Phase 1 audit that forked this sub-ticket.
