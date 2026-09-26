---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
origin: github-issue senara-solutions/mika#1757
created: 2026-08-20
---

# feat(agent): Emit ToolCallStart/ToolCallResult from process_tool_calls (#1757)

## Summary

mika#1731 shipped the wire (`StreamEvent::ToolCallStart` / `ToolCallResult`
variants, `StreamEventSender` type alias, `AgentParams.stream_tx` field,
forward-compat catch-all) but left the emission plumbing dormant. Subscribers
of `/a2a/{agent}` streaming can already parse the frames — nothing ever
publishes them. mika#1727 (TUI thin-client) needs live tool-call receipts to
land; the mika-arch verdict receipts and the operator dashboard both benefit
from real-time tool-call visibility. This plan wires emission at the single
choke point — `process_tool_calls` in `tool_execution/dispatch.rs` — so every
physical tool dispatch emits one `ToolCallStart` + one `ToolCallResult`
frame, with the per-turn dedup replay path (mika#582) staying silent and
zero regressions in existing behaviour.

Same shape as mika#1758 (`TaskEventFrame` emission) shipped in PR#1928
tonight: wire-first ticket landed the enum + broadcaster, this ticket lands
the producer, tests assert the exact frame sequence via a live subscriber.

---

## Problem Frame

**What breaks:** `StreamEvent::ToolCallStart` and `ToolCallResult` are never
constructed. The A2A `handle_message_stream` handler subscribes clients to
a broadcast channel that never receives these frames. The TUI thin-client
(mika#1727) is blocked pending emission — without it, "running Bash: X"
receipts cannot render live; only `StatusUpdate` and final `Message` frames
fire during a run.

**Why the wire went dormant:** mika#1731 shipped as a "wire-first split" —
the enum variants, sender type alias, `AgentParams.stream_tx` field, and
forward-compat `Unknown` catch-all landed in PR#1756. The follow-up producer
was deferred to this ticket to keep the wire PR reviewable. Sibling parallel:
mika#1741 shipped `PermissionStreamFrame` the same way.

**Where the emission belongs:** `crates/mika-agent/src/tool_execution/dispatch.rs`
— `process_tool_calls()`. This function is the sole physical-dispatch site
for tools (`execute_tool()` at line 166). It already handles the per-turn
dedup guard (`dedup_cache: HashMap<(String, String), ToolOutput>`); emitting
inside the `else` branch (fresh dispatch, not a cached replay) satisfies the
"one Start + one Result per physical dispatch" invariant automatically.

**What must be threaded:** the `stream_tx` field on `AgentParams` today
carries only a `Option<StreamEventSender>`. Frames also need `task_id` (the
A2A task correlation) and `context_id` (optional A2A context). Both are known
at the a2a.rs `handle_message_stream` construction site (line 347, 348) but
not currently threaded past `AgentParams.stream_tx`. The thread must reach
`process_tool_calls` through `run_loop`.

---

## Requirements

- **R1.** `ToolCallStart` frame emitted immediately BEFORE each physical
  `execute_tool()` call in `process_tool_calls`. Frame carries: `task_id`,
  optional `context_id`, `step` (u32), `tool_name`, `args_summary`
  (truncated), `timestamp` (RFC3339 UTC).
- **R2.** `ToolCallResult` frame emitted immediately AFTER `execute_tool()`
  returns (before `save_tool_call`, before summary construction). Frame
  carries: same identifiers, `success`, `non_zero_exit`, `output_summary`
  (truncated), `duration_ms` (from `tool_start.elapsed()`), `timestamp`.
- **R3.** Per-turn dedup replay path (`if let Some(cached)` branch at
  dispatch.rs:133–149) MUST NOT emit either frame. Consumers see one
  Start + one Result per **physical** dispatch, matching the `tool_calls`
  table's row-per-physical-dispatch shape.
- **R4.** Send-message turn-boundary suppression path (dispatch.rs:92–127)
  MUST NOT emit — the tool never physically dispatched.
- **R5.** Emission is fire-and-forget: a `broadcast::Sender::send` returning
  `Err` (zero live subscribers) is not an error and does NOT fail the tool
  call. No panic on serde or channel failure. Errors logged at `debug!`.
- **R6.** Emission is opt-in per turn: only turns with an attached
  `stream_ctx` (i.e., A2A `message/stream` today) emit frames. CLI, silent,
  team, delegate, and A2A `message/send` turns pass `None` and emit nothing.
- **R7.** Truncation caps: `args_summary` and `output_summary` use
  `mika_a2a::streaming::truncate_tool_summary` (UTF-8 safe, 500-char cap).
  Frames are glanceable receipts; full call bodies stay queryable via
  `GET /api/v1/traces/:trace_id/tool-calls`.
- **R8.** Zero regressions to the existing `process_tool_calls` behaviour:
  dedup semantics, image budget accounting, secret scrubbing, tool-call
  persistence, send-message turn boundary, and all summary construction
  proceed identically. Emission is an additive side effect.
- **R9.** Test coverage: injection-verified per
  `feedback_verify_pipeline_passes_without_the_fix`. A baseline (no fix)
  test must fail; the shipped fix makes it pass.

---

## Product Contract preservation

No prior brainstorm/Product Contract exists — this plan is
`product_contract_source: ce-plan-bootstrap`. The ticket body (mika#1757) is
the authoritative product input; its constraints (dedup silence, fire-and-
forget) are transcribed verbatim into `## Acceptance criteria` below.

---

## Key Technical Decisions

### KTD1. Bundle `sender + task_id + context_id` into `ToolCallStreamContext`

**Choice:** Replace `AgentParams.stream_tx: Option<StreamEventSender>` with
`AgentParams.stream_ctx: Option<Arc<ToolCallStreamContext>>`, where
`ToolCallStreamContext` is a new struct in `mika_a2a::streaming` bundling
`{ sender: StreamEventSender, task_id: String, context_id: Option<String> }`
with two emit helpers (`emit_start`, `emit_result`).

**Rationale:**
- Frames need `task_id` (required) and `context_id` (optional). Passing them
  as three separate `Option<T>` fields on `AgentParams` fragments an
  invariant that must always be consistent — "if you have a sender you have
  the ids the frames need."
- Emit helpers on the context centralise timestamp capture + truncation +
  fire-and-forget wrapping. The `process_tool_calls` call site then reads
  as `stream_ctx.emit_start(step, tool_name, args_summary)` — one line, no
  ambient dependencies.
- `Arc<>` wrapping keeps `AgentParams` `Clone`-friendly for the two
  callsites that construct it (`server::a2a::run_a2a_agent`,
  `server::handlers::handle_message`); the context lives for the duration
  of a single agent turn and needs to survive re-entry into `run_loop`.
- Zero new fields to add to `run_loop`'s already-30-arg signature beyond
  the one we must add.

**Alternatives rejected:**
- Keep `stream_tx: Option<StreamEventSender>` and add sibling fields
  `stream_task_id: Option<String>`, `stream_context_id: Option<String>`:
  three fields with an implicit "must-all-be-Some-together" contract.
  Prime failure vector for future call sites (silent, team) accidentally
  populating one without the others.
- Emit from `execute_tool()` (deeper in the stack): would need to thread
  `stream_ctx` through per-invocation, and `execute_tool` doesn't know
  the `step` index. Emitting at `process_tool_calls` is the canonical
  choke point.
- Pass a closure `Fn(StreamEvent) -> ()` instead of a context: erases
  compile-time proof that emission preserves the frame contract, forces
  every emit site to reconstruct the frame from scratch (frame variant
  drift risk).

### KTD2. Emit before/after `execute_tool()` at the physical-dispatch site

**Choice:** In the `else` branch of the dedup check (dispatch.rs:150–265),
call `stream_ctx.emit_start(step, name, &input_summary)` immediately
before the `tool_start = Instant::now()` line, and
`stream_ctx.emit_result(step, name, tool_succeeded, non_zero_exit,
&output_summary, tool_latency_ms)` immediately after `tool_latency_ms`
is computed but before the `save_tool_call` DB write.

**Rationale:**
- The `else` branch is exactly the "physical dispatch" gate — the `if let
  Some(cached)` branch is per-turn dedup replay and MUST NOT emit (R3).
- Positioning Start before `tool_start` and Result after `elapsed()`
  captures the true wall-clock latency the consumer wants to render.
- Positioning Result BEFORE `save_tool_call` matches the "receipts are
  glanceable, DB is authoritative" contract (per mika#1731 comment on
  `ToolCallResultEvent`). If the DB write times out or fails, the wire
  frame still fires — consumers get a live signal, dashboard fetches the
  authoritative row later.
- All computed values needed for the Result frame (`tool_succeeded`,
  `non_zero_exit`, `output_summary`, `tool_latency_ms`) are already
  in-scope at the emission site. No re-computation, no ambient lookups.

**Alternatives rejected:**
- Emit only Start OR only Result: half-emit breaks the pair contract
  (`start + result` matches `execute_tool`'s wall-clock semantics; either
  alone is useless for a UI).
- Emit from `execute_tool()`: it doesn't have `step`, doesn't compute
  the summary shape, would need to reconstruct dedup-branch information
  the caller already computed.
- Emit inside the dedup-hit branch too: violates R3 explicitly. TUI would
  display N duplicate receipts for one physical dispatch.

### KTD3. Fire-and-forget emit helper on the context

**Choice:** Both `emit_start` and `emit_result` methods on
`ToolCallStreamContext` return `()`. Internally: construct the frame,
call `sender.send(frame)`, discard the returned `Result` (log at `debug!`
if error).

**Rationale:**
- Matches mika#1731's comment contract on `AgentParams.stream_tx`:
  "Emission is fire-and-forget: broadcast errors are `debug!`-logged and
  do not fail the tool call."
- `tokio::sync::broadcast::Sender::send` is synchronous and non-blocking
  (returns `Err` only when zero subscribers exist). No `.await` needed.
- Emit surface stays a single line at each call site — no error branching
  contaminates `process_tool_calls`.

### KTD4. Timestamp captured at emit time, per-frame

**Choice:** Each emit helper captures its own timestamp via
`chrono::Utc::now().to_rfc3339()` — Start captures at the pre-dispatch
moment, Result captures at the post-dispatch moment. `duration_ms` on
Result comes from the caller's already-computed `tool_start.elapsed()`.

**Rationale:**
- Two independent timestamps let a consumer compute end-to-end wall-clock
  latency including LLM→engine delivery time (Start ts diff to LLM turn
  start) if desired. `duration_ms` is the pure tool-execution latency.
- Captured at the emit site (not the caller) means the frame construction
  invariant is centralised — no drift risk from callers forgetting to pass
  a timestamp.

### KTD5. Thread `stream_ctx: Option<&Arc<ToolCallStreamContext>>` through `run_loop`

**Choice:** Add one `Option<&Arc<ToolCallStreamContext>>` parameter to
`run_loop` after the existing `long_running_ctx` argument. Pass it verbatim
into both `process_tool_calls` sites (line ~985 and line ~2390).

**Rationale:**
- Minimal signature growth: `run_loop` is already 22-arg with
  `#[allow(clippy::too_many_arguments)]`; one more is not a hazard.
- Threading via `&Arc<T>` preserves the "cheap to clone, but don't clone
  unless you must" contract.
- The three `run_loop` callers (`run_agent_inner`, `run_silent_inner`,
  `run_team_agent_inner_impl`) each thread the field explicitly — silent
  and team pass `None`, conversation pulls from `params.stream_ctx`.

**Alternatives rejected:**
- Put `stream_ctx` on `ToolContext` (already threaded to `execute_tool`):
  `ToolContext` is per-execution, not per-loop. `process_tool_calls` never
  reaches into `ToolContext` for emit-relevant state. Adding it there
  would still require reaching into `stream_ctx` from within
  `process_tool_calls` — same shape, wrong home.
- Global broadcast registry keyed by task_id: adds a `DashMap` shared with
  a2a.rs and turns the context lookup into a runtime failure vector when
  the key is missing. Explicit thread stays checked at compile time.

### KTD6. `handle_message_send` continues to pass `None`

**Choice:** `server::a2a::handle_message_send` (non-streaming JSON-RPC
path) keeps `stream_ctx: None`. Only `handle_message_stream` constructs
and passes a real `ToolCallStreamContext`. `server::handlers::handle_message`
(POST /message, gateway path) also stays `None`.

**Rationale:**
- Non-streaming callers have no subscriber to broadcast to. Passing a
  ctx would allocate a broadcaster with zero subscribers and waste
  cycles on N pointless serialise-and-drop cycles per turn.
- Gateway `/message` is Telegram-shape (single-shot response). If a future
  ticket wants tool-call receipts there, it constructs a per-turn
  broadcaster the same way `handle_message_stream` already does.
- Aligns with mika#1731's original design comment: "The A2A
  `handle_message_stream` handler populates this with the per-task
  broadcaster it already owns; all other callers (CLI, silent, team,
  delegate) pass `None`."

### KTD7. Test strategy: unit tests in `dispatch.rs` + integration test in `crates/mika-agent/tests/`

**Choice:** Two layers.

Unit tests in `#[cfg(test)] mod tests` inside `tool_execution/dispatch.rs`:
build a minimal test harness (mock `ToolRegistry` with a builtin tool that
returns a scripted `ToolOutput`, in-memory `AsyncDatabase`, real broadcast
channel + subscriber), drive `process_tool_calls` directly, assert:
- One `ToolCallStart` + one `ToolCallResult` fired for a single physical
  tool call; fields carry expected values.
- Dedup case (two identical tool_use blocks in one response): exactly
  one Start + one Result (not two).
- Zero-subscriber case (drop the receiver before call): no panic, no
  side-effect regression.
- `stream_ctx = None`: no frames, no allocations, semantics unchanged.
- Send-message turn-boundary suppression: emits no frames for the
  suppressed call.

Integration test at `crates/mika-agent/tests/tool_call_stream.rs`:
end-to-end verifiable via `EvalHarness` + `MockLlmProvider`, scripts a
turn calling one deterministic builtin (e.g. `store_fact`), asserts the
frame sequence on a live subscriber. Same shape as the sibling
`tests/task_event_stream.rs` (mika#1758) shipped tonight.

**Rationale:**
- Unit tests give surgical coverage of the emission logic (dedup path,
  suppressed path, error swallow). Fast and hermetic.
- Integration test proves the whole thread (`AgentParams.stream_ctx` →
  `run_loop` → `process_tool_calls` → broadcaster → subscriber) is wired.
  Same pattern that shipped mika#1758 tonight.
- Injection-verified: each test's pre-fix baseline is a compile-time
  proof that the assertions catch the regression class.

---

## Definition of Done

- [ ] `mika_a2a::streaming::ToolCallStreamContext` struct added, bundling
      sender + task_id + optional context_id + `emit_start(step, tool_name,
      &args_summary)` and `emit_result(step, tool_name, success,
      non_zero_exit, &output_summary, duration_ms)` helpers.
- [ ] `AgentParams.stream_tx: Option<StreamEventSender>` renamed to
      `AgentParams.stream_ctx: Option<Arc<ToolCallStreamContext>>`; docstring
      updated.
- [ ] `run_loop` threads `stream_ctx: Option<&Arc<ToolCallStreamContext>>`
      to both `process_tool_calls` callsites (dispatch.rs, agent_loop lines
      ~985 and ~2390).
- [ ] `process_tool_calls` accepts the parameter and emits Start + Result
      frames on the physical-dispatch path only (not dedup replay, not
      suppressed send_message).
- [ ] `server::a2a::run_a2a_agent` renamed `stream_tx` param to
      `stream_ctx`; `handle_message_stream` constructs the context from the
      per-task broadcaster + task_id + context_id.
- [ ] `server::a2a::handle_message_send` continues to pass `None`.
- [ ] `server::handlers::handle_message` continues to pass `None`.
- [ ] Unit tests in `dispatch.rs::tests`: physical dispatch fires both
      frames; dedup replay silent; zero subscribers silent; ctx=None
      silent; suppressed send_message silent.
- [ ] Integration test at `tests/tool_call_stream.rs`: end-to-end via
      `EvalHarness`, asserts frame sequence on a live subscriber.
- [ ] `cargo build --release`, `cargo test`, `cargo clippy`, `cargo fmt
      --check` all clean.
- [ ] PR body carries `Closes #1757` and calls out the mika#1727
      unblock.

---

## Acceptance criteria

Transcribed from the issue body:

- [ ] **AC1** — `process_tool_calls` in
      `crates/mika-agent/src/tool_execution/dispatch.rs` emits
      `ToolCallStart` before each physical tool dispatch and
      `ToolCallResult` after.
- [ ] **AC2** — Emission is fire-and-forget: broadcast errors are
      `debug!`-logged and do not fail the tool call.
- [ ] **AC3** — Per-turn dedup replays (`ToolContext` cached
      `ToolOutput`) do NOT emit frames.
- [ ] **AC4** — Threading of `stream_ctx` reaches `process_tool_calls`
      through `run_agent()` → `run_agent_inner()` → `run_loop()` and the
      two `process_tool_calls` callsites within `run_loop`, plus the
      sibling silent (`run_silent_agent`) and team (`run_team_agent`)
      loops.
- [ ] **AC5** — Integration test at
      `crates/mika-agent/tests/tool_call_stream.rs` uses `EvalHarness`
      + `MockLlmProvider` with a scripted turn calling one deterministic
      builtin; asserts the frame sequence on a live subscriber.

---

## Verification Contract

**Test class:** unit + integration (in-crate).

**Deterministic invariants gated by CI:**
1. A `MockLlmProvider` turn calling `store_fact` on an `EvalHarness` with
   an attached broadcaster and one subscriber yields exactly one
   `ToolCallStart` frame + one `ToolCallResult` frame, in order, with
   matching `task_id`, `tool_name = "store_fact"`, monotonic timestamps,
   `duration_ms >= 0`, `success = true`.
2. A turn with two identical `store_fact(key=X, value=Y)` tool_use blocks
   in a single response yields ONE Start + ONE Result frame (dedup replay
   path silent).
3. A turn on an `EvalHarness` with no broadcaster attached
   (`stream_ctx = None`) yields no frames on any subscriber.
4. A turn on an `EvalHarness` with a broadcaster + a subscriber dropped
   before the call completes without panic; the agent still returns
   normally.
5. Conversation-mode second-send_message suppression: turn produces no
   Start/Result frames for the suppressed call (only for the successful
   first call).

**Non-gated observability (manual):** attach `mika dev tool-call-stream`
(future ticket — not in scope) to a running mika-spirit and observe frames
in real-time. This ticket does NOT ship a CLI consumer; the frame surface
is validated via the integration test alone.
