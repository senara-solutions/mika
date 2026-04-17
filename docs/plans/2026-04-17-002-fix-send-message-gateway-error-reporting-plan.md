---
title: "fix: Surface gateway non-2xx responses as send_message tool errors"
type: fix
status: active
date: 2026-04-17
---

# fix: Surface gateway non-2xx responses as send_message tool errors

## Overview

The `send_message` tool always reports `success=true` / `"Message sent."` regardless of whether the gateway actually delivered the message. When the gateway returns non-2xx (e.g., 502), `GatewayMessageSender::send()` saves to `failed_sends` and returns `Ok(())`, hiding the failure from the agent. This fix makes the tool surface delivery failures as `ToolOutput::error` so agents can detect and respond to them.

## Problem Frame

Issue #581: mika-dev sent a sprint notification via `send_message`. The gateway returned 502, but the tool reported `success=1` / `"Message sent."`. The agent proceeded confidently, informed nobody of the failure, and had no path to retry. Every future gateway/channel failure would be equally invisible — this is the class fix.

Institutional learnings confirm the design principle: HTTP status codes ARE well-defined errors (unlike exec handler exit codes) and must surface as `ToolOutput::error`, not `ToolOutput::success`. The grounding rule is the secondary defense; the primary defense must be at the tool output level.

## Requirements Trace

- R1. Non-2xx gateway response → `ToolOutput::error` with status code and body snippet
- R2. 2xx gateway response → `ToolOutput::success("Message sent.")` (unchanged)
- R3. Network/timeout errors → `ToolOutput::error` with classification
- R4. `failed_sends` queue continues to be populated for server-side retry flush
- R5. Regression tests cover 502 (failure) and 200 (success) scenarios

## Scope Boundaries

- Only the agent-side `send_message` tool and `GatewayMessageSender` are changed
- Gateway-side behavior is not modified (companion issue for root cause of 502)
- The `MessageSender` trait signature changes from `Result<()>` to `Result<SendOutcome>`
- Retry logic (one retry after 2s) is preserved — errors surface only after both attempts fail

### Deferred to Separate Tasks

- Root cause of gateway 502 (chat_id=0 routing): separate issue
- Silent mode dead-end handling when delivery channel itself fails: accepted for now — step limit bounds waste

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/messaging.rs` — `MessageSender` trait, `GatewayMessageSender` with `try_send()` retry logic
- `crates/mika-agent/src/tools/send_message.rs` — `SendMessageTool::execute()` uses `sender.send().await?`
- `crates/mika-agent/src/tools/a2a_call.rs:131-138` — reference pattern: catches `Err` and returns `ToolOutput::error(format!(...))`
- `crates/mika-agent/src/tools/mod.rs` — `ToolOutput` struct with `success()`/`error()` constructors
- `crates/mika-agent/src/server/handlers.rs` — failed_sends flush logic (up to 5 pending per inbound message)

### Institutional Learnings

- `docs/solutions/logic-errors/tool-call-success-contradicts-non-zero-exit.md` — exact same pattern: derived `success` field didn't consider actual outcome
- `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md` — explicit carve-out: "HTTP status codes are well-defined errors" — do NOT follow exec handler pattern
- `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` — grounding rule is secondary defense; primary defense must be tool-level accuracy
- `docs/solutions/integration-issues/gateway-inbound-webhook-retry-on-429-5xx.md` — gateway uses `ForwardResult` enum with `Success`/`Retryable`/`Permanent` classification

## Key Technical Decisions

- **Richer return type over `Err` propagation:** `MessageSender::send()` returns `Result<SendOutcome>` (enum: `Delivered` / `Failed { reason: String }`) instead of `Result<()>`. This lets the tool distinguish "delivery confirmed" from "delivery failed, queued for retry" without breaking the trait's error semantics. `Err` remains reserved for infrastructure failures (chat_id resolution, DB errors). The `?` operator in the tool continues to work for infra errors; the `match` on `SendOutcome` handles delivery results.
- **Keep `failed_sends` AND return `Failed`:** Belt-and-suspenders. The server-side flush logic still works. The agent also knows delivery failed and can inform the user or retry.
- **Error message includes status code:** Following the gateway's own classification taxonomy, the error message distinguishes connection errors, HTTP status codes, and timeouts so the agent can make informed decisions.

## Open Questions

### Resolved During Planning

- **Should the trait return `Err` or a richer type?** Richer type (`SendOutcome`). Returning `Err` after saving to `failed_sends` creates ambiguity — the tool's `?` operator would propagate it as an `anyhow::Error`, which the agent loop treats differently from `ToolOutput::error`. A `SendOutcome` enum keeps the success/failure decision at the tool level where it belongs.
- **Should `failed_sends` still be populated?** Yes. The existing flush logic in `server/handlers.rs` provides automatic retry on next inbound message. Dropping it would make the agent solely responsible for retry.
- **What about `resolve_chat_id` errors?** These are config errors, not delivery errors. They continue to propagate via `?` as before — they're infrastructure failures, not gateway response failures.

### Deferred to Implementation

- Exact error message wording — directional: include HTTP status code and truncated body (first ~200 chars)
- Whether `MockSender` needs additional builder methods or if a simple closure-based approach suffices

## Implementation Units

- [x] **Unit 1: Add `SendOutcome` enum and update `MessageSender` trait**

**Goal:** Change the `MessageSender` trait to return `Result<SendOutcome>` instead of `Result<()>`, where `SendOutcome` distinguishes successful delivery from queued-after-failure.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/messaging.rs`
- Test: `crates/mika-agent/src/messaging.rs` (inline tests)

**Approach:**
- Define `SendOutcome` enum with `Delivered` and `Failed { reason: String }` variants
- Update `MessageSender::send()` return type to `Result<SendOutcome>`
- In `GatewayMessageSender::send()`: after both retries fail, save to `failed_sends` AND return `Ok(SendOutcome::Failed { reason })` instead of `Ok(())`
- In `try_send()`: capture the response body (truncated) along with status code for the error message
- On success path: return `Ok(SendOutcome::Delivered)`
- Classify errors: connection (`is_connect()`), timeout (`is_timeout()`), HTTP status (from `try_send` bail)

**Patterns to follow:**
- Gateway's `ForwardResult` enum pattern (Success/Retryable/Permanent)
- `a2a_call.rs` error message format

**Test scenarios:**
- Happy path: `resolve_chat_id` with explicit override returns `Delivered` (existing test adapted)
- Happy path: `resolve_chat_id` from DB returns correct ID (existing test preserved)
- Error path: `resolve_chat_id` with no config returns `Err` (existing test preserved, still an infra error)

**Verification:**
- `MessageSender` trait compiles with new return type
- All existing `messaging.rs` tests pass (adapted for `SendOutcome`)

- [x] **Unit 2: Update `SendMessageTool` to surface delivery failures**

**Goal:** Replace the `?` operator on `sender.send()` with a `match` that converts `SendOutcome::Failed` to `ToolOutput::error`.

**Requirements:** R1, R2, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/send_message.rs`
- Test: `crates/mika-agent/src/tools/send_message.rs` (inline tests)

**Approach:**
- Replace `sender.send(&cleaned).await?` with a `match` on the `Result<SendOutcome>`
- `Ok(SendOutcome::Delivered)` → `ToolOutput::success("Message sent.")`
- `Ok(SendOutcome::Failed { reason })` → `ToolOutput::error(format!("Message delivery failed: {reason}"))`
- `Err(e)` → `ToolOutput::error(format!("Message delivery error: {e}"))` (infra failures like chat_id resolution)
- Update `MockSender` to support configurable outcomes (return `SendOutcome` instead of always `Ok(())`)

**Patterns to follow:**
- `a2a_call.rs:131-138` — catches `Err` and returns `ToolOutput::error(format!(...))`

**Test scenarios:**
- Happy path: sender returns `Delivered` → `ToolOutput::success` with "Message sent." and `is_error: false`
- Error path: sender returns `Failed { reason: "gateway /send returned 502 Bad Gateway" }` → `ToolOutput::error` with `is_error: true` and reason in content
- Error path: sender returns `Failed { reason: "connection refused" }` → `ToolOutput::error` with network classification
- Error path: sender returns `Err` (infra failure, e.g., chat_id not configured) → `ToolOutput::error` with infra error message
- Happy path: no sender configured → `ToolOutput::success` with "NOT delivered" warning (unchanged behavior)
- Edge case: empty text after stripping → `ToolOutput::success` with processing message (unchanged)

**Verification:**
- `send_message` tool returns `is_error: true` when sender reports failure
- `send_message` tool returns `is_error: false` when sender reports delivery
- All existing tool tests pass (adapted for new `MockSender`)

- [x] **Unit 3: Integration test via eval harness**

**Goal:** Add an eval harness test that verifies the agent receives a failure signal when `send_message` encounters a gateway error.

**Requirements:** R5

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/tests/eval/test_tool_calling.rs`
- Read: `crates/mika-agent/tests/eval/harness.rs` (for `EvalHarness` builder API)

**Approach:**
- Add a test using `EvalHarness` with `MockLlmProvider` that exercises the `send_message` tool with a failing sender
- The `EvalHarness` builder may need a method to inject a custom `MessageSender` — check existing builder API first
- Verify the tool_call record in the DB has `success=0`

**Execution note:** Check `EvalHarness` API for sender injection support before writing the test. If not supported, the unit test coverage from Unit 2 is sufficient and this unit pivots to adding sender injection to the harness.

**Patterns to follow:**
- Existing eval tests in `test_tool_calling.rs` — especially send_message dedup tests

**Test scenarios:**
- Integration: agent calls `send_message` with failing sender → tool_call DB record has `success=false` and output contains error message
- Integration: agent calls `send_message` with succeeding sender → tool_call DB record has `success=true` and output is "Message sent."

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- New test exercises the full `run_agent()` path with `send_message` failure

## System-Wide Impact

- **Interaction graph:** `send_message` tool → `MessageSender::send()` → `failed_sends` table → server flush logic. The flush logic is unaffected (still reads from `failed_sends`). Tool callers now see failure when delivery fails.
- **Error propagation:** Delivery failures flow as `SendOutcome::Failed` (not `Err`) through the tool to the agent as `ToolOutput::error`. Infrastructure failures (`Err`) are caught at the tool level and also surface as `ToolOutput::error`.
- **State lifecycle risks:** None — `failed_sends` continues to be populated. No new state is introduced.
- **API surface parity:** No other tools use `MessageSender`. The `send` method signature change is internal.
- **Unchanged invariants:** The `None` sender path (no outbound configured) remains unchanged — intentionally returns success with warning text. The `failed_sends` flush logic in `server/handlers.rs` is unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Agent retry loops when gateway is persistently down | Step limit (`SilentTrigger::max_steps()`) bounds the waste. Continuation turn fallback handles exhaustion. The agent sees the error and can choose to inform the user rather than blindly retry. |
| Breaking other `MessageSender` implementations | Only two impls: `GatewayMessageSender` (updated) and test `MockSender` (updated). No external consumers. |
| `try_send` response body read adds latency | Body is only read on error path (non-2xx). Truncated to ~200 chars. Negligible overhead. |

## Sources & References

- **Issue:** [#581 — send_message tool reports success on gateway non-2xx responses](https://github.com/senara-solutions/mika/issues/581)
- Related code: `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/tools/send_message.rs`
- Related learnings: `docs/solutions/logic-errors/tool-call-success-contradicts-non-zero-exit.md`
- Related learnings: `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md`
