---
title: "fix: Handle chat_id == 0 in send_message as structured NoChannel outcome"
type: fix
status: active
date: 2026-04-20
---

# fix: Handle chat_id == 0 in send_message as structured NoChannel outcome

## Overview

`send_message` invoked with `chat_id == 0` (a documented sentinel for "no reply channel") propagates the gateway's 400 error as a generic failure. The agent retries uselessly, pollutes `failed_sends`, and the LLM sees an opaque error instead of an actionable signal to pivot to channel-appropriate tools (e.g., `run_gh`). The fix adds a `SendOutcome::NoChannel` variant detected before the HTTP POST, returns a clear tool output, and adds gateway observability.

## Problem Frame

`chat_id == 0` is the GitHub webhook design's sentinel meaning "this session has no Telegram reply channel." The gateway correctly rejects it with 400. But the agent-side `GatewayMessageSender::send()` doesn't recognize this as a permanent, expected condition — it retries (wasting 2s), saves to `failed_sends` (futile), and returns `SendOutcome::Failed` which the tool surfaces as `ToolOutput::error`. The LLM sees "Message delivery failed: gateway /send returned 400: chat_id must be non-zero" — an infrastructure error message, not a redirect to the right tool.

10 occurrences in 24 hours across callbacks, heartbeats, and CLI sessions confirm this is a steady-state defect, not a one-off.

## Requirements Trace

- R1. `send_message` with `chat_id == 0` returns a structured "no reply channel" result to the LLM (not a gateway 400 propagation)
- R2. Tool output tells the LLM what to use instead (e.g., `run_gh` for GitHub channels)
- R3. No retry attempt and no `failed_sends` entry for `chat_id == 0` (permanent condition)
- R4. Gateway adds observability for `chat_id == 0` POST arrivals (defense-in-depth logging)
- R5. Unit test: `send_message` with `chat_id == 0` → typed `Ok(NoChannel)`, not `Err` or `Failed`
- R6. Eval harness integration test covering the flow
- R7. All existing `SendOutcome` match sites compile and handle the new variant

## Scope Boundaries

- No changes to the `MessageSender` trait signature (still returns `Result<SendOutcome>`)
- No channel field plumbed into `ToolContext` — the tool output is actionable without naming the channel (the LLM already knows channel context from the system prompt)
- No per-channel `MessageSender` trait selection (explicitly deferred per issue #650)
- No changes to how `chat_id` flows from gateway to agent in `/message` handler
- No schema migration needed

### Deferred to Separate Tasks

- Per-channel `MessageSender` trait selection (long-term, out of scope per issue description)
- `resolve_chat_id()` rejecting `0` from DB (defense-in-depth improvement, but the `send()` check catches it first and is the correct architectural layer for the fix)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/messaging.rs` — `SendOutcome` enum (line 10-20), `GatewayMessageSender::send()` (line 134-175), `resolve_chat_id()` (line 78-89)
- `crates/mika-agent/src/tools/send_message.rs` — tool handler matching on `SendOutcome` (line 64-98), `None` sender pattern at line 90-96 (returns `ToolOutput::success` with warning — the closest precedent for NoChannel)
- `crates/mika-agent/src/server/handlers.rs` — failed_sends flush (line 846-858), end-of-turn notification (line 906-910)
- `crates/mika-agent/src/server/verdict_handler.rs` — verdict notification (line 307-308)
- `crates/mika-agent/src/server/ci_success_handler.rs` — CI notification (line 311-312)
- `crates/mika-agent/src/task_engine/dispatcher.rs` — fire-and-forget task dispatch (line 145-149)
- `crates/mika-gateway/src/routes.rs` — existing `chat_id == 0` guard (line 826-831)
- Test at `messaging.rs:281` — `test_resolve_chat_id_poisoned_zero_in_db` documents current pass-through behavior

### Institutional Learnings

- `docs/solutions/logic-errors/send-message-tool-false-success-on-gateway-error.md` — Documents the `SendOutcome` enum introduction. All callsites must handle new variants.
- `docs/solutions/integration-issues/github-webhook-poisons-telegram-chat-id.md` — Documents the `chat_id == 0` sentinel and the gateway's 400 validation. The fix here is the agent-side companion.
- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — Documents `resolve_chat_id()` mechanism and delegate chat_id override flow.

## Key Technical Decisions

- **Check in `GatewayMessageSender::send()`, not in the tool handler:** The `send()` method is the single choke point called by the tool, task engine dispatcher, verdict/CI handlers, and failed_sends flush. Checking here catches all callers uniformly, avoiding scattered duplicate checks. The tool handler only needs to match the new `NoChannel` variant.

- **New `SendOutcome::NoChannel` variant (not a separate pre-check method):** Adding a variant to the existing enum is clean for the type system and forces exhaustive handling at every match site (Rust compiler enforces this). The alternative — a `can_deliver()` pre-check — would be optional to call and easy to forget.

- **`ToolOutput::success` for NoChannel (not `ToolOutput::error`):** Following the existing "no sender" precedent at line 90 of `send_message.rs`. `chat_id == 0` is a permanent session condition, not a transient error. Using `is_error: true` would cause the LLM to retry in a loop. The success-with-warning pattern gives the LLM a clear signal without triggering retry behavior.

- **Skip `failed_sends` for NoChannel:** `chat_id == 0` is a permanent condition — saving to `failed_sends` creates futile retry entries that will fail again on flush. The `send()` method returns `Ok(NoChannel)` before reaching the retry/save logic.

- **Message persistence ordering unchanged:** The `send_message` tool persists to conversation history at line 60 before calling the sender. Changing this ordering is a separate concern. The message was "said" by the agent even if it couldn't be delivered — keeping it in history is consistent with the `SendOutcome::Failed` behavior where the message is also persisted.

## Implementation Units

- [x] **Unit 1: Add `SendOutcome::NoChannel` variant and detect in `GatewayMessageSender::send()`**

**Goal:** Add the typed variant and short-circuit before HTTP POST when `chat_id == 0`.

**Requirements:** R1, R3, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/messaging.rs`
- Test: `crates/mika-agent/src/messaging.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `NoChannel` variant to `SendOutcome` enum with a doc comment explaining the sentinel
- In `GatewayMessageSender::send()`, after `resolve_chat_id()` returns `Ok(id)`, check `if id == 0` and return `Ok(SendOutcome::NoChannel)` immediately — before the payload construction, HTTP POST, retry, and `failed_sends` write
- This naturally satisfies R3 (no retry, no failed_sends) because the early return skips all downstream logic
- Update `test_resolve_chat_id_poisoned_zero_in_db` — the test currently documents pass-through behavior; update the comment to note that `send()` now catches this before the HTTP POST
- Add a new test: construct `GatewayMessageSender` with explicit `chat_id: Some(0)`, call `send()`, assert `Ok(SendOutcome::NoChannel)`

**Patterns to follow:**
- Existing `SendOutcome::Delivered` and `SendOutcome::Failed` variant style
- The existing `resolve_chat_id()` error early-return pattern at line 136

**Test scenarios:**
- Happy path: `GatewayMessageSender::send()` with `chat_id == 0` (explicit override `Some(0)`) returns `Ok(SendOutcome::NoChannel)` without making any HTTP request
- Happy path: `GatewayMessageSender::send()` with `chat_id == 0` from DB lookup (poisoned "0" in `customer_config`) returns `Ok(SendOutcome::NoChannel)`
- Edge case: `GatewayMessageSender::send()` with valid non-zero `chat_id` still returns `Delivered` (regression guard — use a mock HTTP server or accept that this is covered by existing tests)
- Edge case: `resolve_chat_id()` returning `Err` (no chat_id configured) still propagates as `Err` (not `NoChannel`)

**Verification:**
- `cargo test -p mika-agent messaging` passes with new and updated tests
- `cargo check -p mika-agent` confirms all `SendOutcome` match sites report exhaustiveness errors (will be fixed in Unit 2)

---

- [x] **Unit 2: Handle `SendOutcome::NoChannel` at all match sites**

**Goal:** Update every `SendOutcome` match arm across the codebase so the project compiles and each callsite handles `NoChannel` appropriately.

**Requirements:** R2, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/send_message.rs`
- Modify: `crates/mika-agent/src/server/handlers.rs`
- Modify: `crates/mika-agent/src/server/verdict_handler.rs`
- Modify: `crates/mika-agent/src/server/ci_success_handler.rs`
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs`
- Test: `crates/mika-agent/src/tools/send_message.rs` (inline tests)

**Approach:**
- **`send_message` tool** (primary — R2): Add `Ok(SendOutcome::NoChannel)` arm returning `ToolOutput::success(...)` with actionable text: "No reply channel for this session (chat_id is zero). The user cannot receive messages via send_message. Use channel-appropriate tools (e.g., run_gh for GitHub) to deliver your response." Follow the existing `None` sender pattern (success, not error)
- **`handlers.rs` failed_sends flush** (~3 match sites): Add `Ok(SendOutcome::NoChannel)` arm with `warn!` log and continue (same as `Failed` but with distinct log message noting permanent condition)
- **`verdict_handler.rs`** and **`ci_success_handler.rs`**: Add `Ok(SendOutcome::NoChannel)` arm with `warn!` log — notifications that can't be delivered are logged but don't block the verdict/CI flow
- **`task_engine/dispatcher.rs`**: Add `Ok(SendOutcome::NoChannel)` arm with `warn!` log — fire-and-forget semantics, same as `Failed`
- **Test mock `MockSender`** in `send_message.rs` and `dispatcher.rs` and `engine.rs`: Add `NoChannel` to any mock implementations that construct `SendOutcome`

**Patterns to follow:**
- The `None` sender arm in `send_message.rs` (line 90-96) for the tool output text style
- The existing `Failed` arm handling at each callsite for the non-tool match sites

**Test scenarios:**
- Happy path: `send_message` tool with `MockSender` returning `SendOutcome::NoChannel` → `ToolOutput` with `is_error: false` and output containing "No reply channel"
- Happy path: `send_message` tool with `MockSender` returning `SendOutcome::Delivered` still works (regression)
- Error path: `send_message` tool with `MockSender` returning `SendOutcome::Failed` still returns `ToolOutput::error` (regression)

**Verification:**
- `cargo build -p mika-agent` compiles cleanly (no unhandled enum variants)
- `cargo test -p mika-agent` passes with new test

---

- [x] **Unit 3: Add gateway observability for `chat_id == 0` arrivals**

**Goal:** Add a structured `warn!` log at the gateway's existing `chat_id == 0` guard so defense-in-depth violations are visible in dashboards.

**Requirements:** R4

**Dependencies:** None (can be done in parallel with Units 1-2)

**Files:**
- Modify: `crates/mika-gateway/src/routes.rs`

**Approach:**
- At the existing `chat_id == 0` check (line 826), add a `warn!` with structured fields (`agent_name`, `request_id`) before returning 400. This makes the event searchable in log aggregation without changing the response.
- Use `tracing::warn!` consistent with gateway logging patterns

**Patterns to follow:**
- Existing `tracing::warn!` usage in `routes.rs` for validation failures
- Structured field style: `warn!(agent_name = ?payload.agent_name, "chat_id == 0 POST received")`

**Test scenarios:**
- Test expectation: none — this is a log-only addition to an existing validation guard. The existing gateway tests for chat_id validation cover the 400 response behavior. Log output verification would require a tracing subscriber test harness which is disproportionate for a single `warn!` line.

**Verification:**
- `cargo build -p mika-gateway` compiles cleanly
- Manual: POST to `/send` with `chat_id: 0` produces structured warn log

---

- [x] **Unit 4: Eval harness integration test**

**Goal:** End-to-end test using `EvalHarness` that verifies the full agent loop handles `send_message` with `NoChannel` correctly.

**Requirements:** R6

**Dependencies:** Units 1, 2

**Files:**
- Modify: `crates/mika-agent/tests/eval/test_tool_calling.rs`
- Test: `crates/mika-agent/tests/eval/test_tool_calling.rs`

**Approach:**
- Create a `MockNoChannelSender` (or reuse `MockSender` with configurable outcome) that returns `Ok(SendOutcome::NoChannel)` on `send()`
- Build `EvalHarness` with `.message_sender(Arc::new(sender))` and mock LLM responses that include a `send_message` tool call
- Assert: tool output contains "No reply channel" text, `is_error` is false, and the LLM receives the actionable message
- Use `assert_tool_output_contains` helper from the eval assertions module

**Patterns to follow:**
- Existing eval tests in `test_tool_calling.rs` for `send_message`
- `EvalHarness::builder().responses(...).message_sender(...).build()` pattern

**Test scenarios:**
- Integration: LLM calls `send_message` with text → sender returns `NoChannel` → tool output is success with "No reply channel" message → agent continues (does not error out or retry)
- Integration: Verify the tool call appears in `trace.tool_calls` with expected output

**Verification:**
- `cargo test -p mika-agent --test eval` passes including the new test
- The test exercises the full `run_agent()` path through `send_message` tool → `MockSender` → `NoChannel` → tool output

## System-Wide Impact

- **Interaction graph:** `SendOutcome` is matched in 7 production sites across 5 files. The Rust compiler's exhaustive match enforcement ensures no site is missed. The `MessageSender` trait signature is unchanged — all `impl MessageSender` blocks are unaffected.
- **Error propagation:** `NoChannel` is an `Ok` variant, not an `Err`. It flows through the same success path as `Delivered` and `Failed`. The `send_message` tool returns `ToolOutput::success` (not error), consistent with the "no sender" precedent.
- **State lifecycle risks:** No schema changes. `failed_sends` table is not written for `NoChannel` (correct — permanent condition). Conversation history is still written (message was "said" by the agent).
- **API surface parity:** The gateway `/send` endpoint response is unchanged. The agent's tool output changes from an error to a success with different text — this is visible to the LLM but not to external APIs.
- **Unchanged invariants:** `MessageSender` trait, `ToolContext` struct, `resolve_chat_id()` return type, gateway `/send` response format, `failed_sends` flush logic (just adds a new match arm).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Exhaustive match enforcement may reveal match sites in untested code paths | Rust compiler catches all sites at build time; each site follows the same pattern as its `Failed` arm |
| LLM may still attempt `send_message` repeatedly despite success result | The "no sender" precedent at line 90 has been stable — LLMs respect `ToolOutput::success` with explanatory text. The text explicitly names an alternative tool. |
| Task engine reminders with `action_type='send_message'` will silently log and continue | This is the correct behavior — same as transient `Failed`. The reminder was for a session that has no reply channel; there's no better fallback without per-channel sender selection (deferred). |

## Sources & References

- Related issue: #650
- Related fixes: #580 (GitHub webhook chat_id poisoning), #524 (verdict handler), #571 (CI success handler)
- Documented solution: `docs/solutions/logic-errors/send-message-tool-false-success-on-gateway-error.md`
- Documented solution: `docs/solutions/integration-issues/github-webhook-poisons-telegram-chat-id.md`
- Documented anti-pattern rules: `docs/solutions/channels/multi-agent-telegram-delivery-and-reply-routing.md`
