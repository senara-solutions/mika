---
title: "feat: Gateway retries inbound webhook delivery on 429/5xx"
type: feat
status: active
date: 2026-04-16
---

# feat: Gateway retries inbound webhook delivery on 429/5xx

## Overview

Add retry-with-backoff to the gateway's GitHub webhook delivery path. When the spawned forwarding task gets HTTP 429 or 5xx from the agent container, it retries with the schedule `[2s, 5s, 15s, 60s, 300s]` before logging an ERROR and dropping the event. No DLQ (separate ticket #590).

## Problem Frame

The gateway returns 200 OK to GitHub immediately, then spawns a task to forward the event to the agent container. If that forwarding fails (429 busy, 5xx transient), the task logs a WARN and silently drops the event. GitHub already got its 200, so it won't retry. The event is permanently lost.

Real-world impact: on 2026-04-15, a `pull_request_review.submitted` event for PR #588 was dropped because mika-dev returned 429 (busy processing a claude-pilot callback). The PR sat APPROVED+green CI but was never auto-merged.

## Requirements Trace

- R1. Retry forwarding on HTTP 429 and 5xx with backoff schedule `[2s, 5s, 15s, 60s, 300s]`
- R2. Do NOT retry on 4xx (other than 429) -- permanent client errors
- R3. Do NOT retry on connection errors (agent offline) -- retries will all fail
- R4. Retry on request timeouts -- transient overload is the most likely cause
- R5. After retry budget exhausted, log ERROR with `delivery_id`, `target_agent`, and last status/error
- R6. Keep `forward_github_event` as a single-attempt function; retry policy in a wrapper
- R7. Document the dedup LRU eviction edge case (late retries may cause duplicates)
- R8. Release semaphore permit during retry sleeps to prevent cross-channel starvation

## Scope Boundaries

- No dead-letter queue or replay CLI (mika#590)
- No LRU TTL changes
- No circuit breaker per agent
- No `Retry-After` header respect (fixed schedule only)
- No Telegram forwarding retry (Telegram has its own retry via `reset_dedup`)
- No replacement of the spawn-per-event model

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/github.rs` -- `forward_github_event()` (lines 573-638), `handle_github_webhook()` spawn task (lines 485-495), delivery cache (lines 322-331)
- `crates/mika-gateway/src/routes.rs` -- `AppState` (lines 84-104), `handle_forward_result()` (lines 467-492), `forward_error_message()` (line 1026)
- `crates/mika-common/src/claude.rs` -- existing retry pattern: `for attempt in 0..=MAX_RETRIES` with `500ms * 2^(attempt-1)` backoff, `is_retryable()` check (lines 452-480, 628-632)
- `crates/mika-common/src/embedding.rs` -- same retry pattern (lines 127-152)

### Institutional Learnings

- **P2-529 tight retry loop** (`docs/solutions/code-review-patterns/async-callbacks-long-running-review-findings.md`): When `dispatch_resume_agent` failed because the agent was busy, the task was immediately re-queued with no delay. Exponential backoff is mandatory from the start.
- **Gateway offline agent classification** (`docs/solutions/ux-improvements/gateway-offline-agent-error-message.md`): Use `reqwest::Error::is_connect()` to distinguish connection errors (agent offline, DNS failure) from transient errors (timeout, broken pipe). Connection errors should NOT be retried.
- **Webhook deferral queue** (`docs/solutions/architecture-patterns/webhook-deferral-queue-callback-sequencing.md`): Agent-side has 60s deferral timeout. Gateway retry window extends well beyond this, which is fine since 429 means the agent rejected the request entirely (not deferred).
- **Duplicate work item on retry** (`docs/solutions/logic-errors/create-work-item-duplicate-on-retry.md`): Retries require idempotency at the receiver. The agent's GitHub message processing is idempotent by design (webhook dedup + work item unique index).
- **HTTP client timeouts** (`docs/solutions/cross-repo-patterns/rust-axum-security-hardening-playbook.md`): Always set both `connect_timeout` and `timeout`. The existing 5s per-request timeout in `forward_github_event` is appropriate.

## Key Technical Decisions

- **Release semaphore during retry sleep**: The 30-permit semaphore is shared between Telegram and GitHub. Holding a permit for up to ~382s during retries would cause cross-channel starvation. Release the permit after each failed attempt, re-acquire before retrying. If the semaphore is full on re-acquire, abandon the retry (the system is legitimately overloaded).
- **Retry timeouts**: `reqwest::Error` timeouts (5s per-request) are retried. Timeouts are more likely caused by transient overload than by the agent having processed-but-not-responded. The small duplicate risk is acceptable given agent-side idempotency.
- **No connection error retry**: `reqwest::Error::is_connect()` (DNS failure, connection refused) means the agent is offline. Retrying with backoff won't help -- the agent needs to be started. Log ERROR immediately.
- **ForwardResult enum**: Change `forward_github_event` from returning `()` to returning a `ForwardResult` enum so the retry wrapper can classify the outcome without duplicating HTTP response parsing.
- **No new dependencies**: Follow the codebase's hand-rolled retry pattern (for loop with backoff) rather than adding a retry crate. The schedule is fixed, not exponential, so a simple slice iteration suffices.

## Open Questions

### Resolved During Planning

- **Should timeouts be retried?** Yes. Timeouts are classified as retryable (R4). The codebase's existing retry patterns in `claude.rs` and `embedding.rs` retry on timeouts. Agent-side idempotency mitigates the small duplicate risk.
- **Should the semaphore be released during sleep?** Yes (R8). Without this, 30 concurrent 429 responses would block ALL webhook processing (Telegram included) for up to 6 minutes. Re-acquiring the permit before each retry attempt provides natural backpressure.
- **Should each retry refresh the LRU cache entry?** No. The delivery_id was inserted before forwarding. Refreshing on each retry would require passing the cache into the retry wrapper, adding complexity for an edge case that only matters under extreme volume (>10k deliveries during a single 300s sleep). Documented as a known edge case per R7.

### Deferred to Implementation

- Exact `ForwardResult` enum variant names -- the important thing is the 4-way classification (success, retryable HTTP, permanent HTTP, network error)
- Whether the retry wrapper lives in `github.rs` or a new `retry.rs` module -- depends on size

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
forward_github_event(state, ...) -> ForwardResult
  |
  +-- Success / Accepted  -> ForwardResult::Success
  +-- HTTP 429 / 5xx      -> ForwardResult::Retryable { status }
  +-- HTTP 4xx (not 429)   -> ForwardResult::Permanent { status }
  +-- reqwest timeout       -> ForwardResult::Retryable { ... }
  +-- reqwest connect error -> ForwardResult::Permanent { ... }
  +-- reqwest other error   -> ForwardResult::Retryable { ... }


handle_github_webhook:
  tokio::spawn(async {
      let _permit = permit;
      deliver_with_retry(
          state, target, text, request_id, repo_name,
          &RETRY_DELAYS,  // [2s, 5s, 15s, 60s, 300s]
      ).await;
  });


deliver_with_retry:
  attempt 0: forward_github_event() using initial permit
  if retryable:
      drop permit
      for delay in RETRY_DELAYS:
          sleep(delay)
          re-acquire permit (try_acquire_owned)
          if no permit available: log WARN, abandon retry
          forward_github_event()
          if success: return
          if permanent: return
          drop permit, continue
      log ERROR (budget exhausted)
```

## Implementation Units

- [x] **Unit 1: Define ForwardResult enum and refactor forward_github_event return type**

**Goal:** Change `forward_github_event` from returning `()` to returning a typed result so the retry wrapper can classify outcomes.

**Requirements:** R1, R2, R3, R4, R6

**Dependencies:** None

**Files:**
- Modify: `crates/mika-gateway/src/github.rs`
- Test: `crates/mika-gateway/src/github.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Define a `ForwardResult` enum with variants for success, retryable error (429/5xx/timeout), and permanent error (4xx/connect). Include the HTTP status code or error description for logging.
- Refactor the existing match arms in `forward_github_event` (lines 613-637) to return the appropriate variant instead of logging and returning `()`.
- Move the logging from `forward_github_event` into the caller (the spawn task) so it can log once after retry exhaustion rather than on every attempt.
- Keep `forward_github_event` as a pure single-attempt function per R6.

**Patterns to follow:**
- `ClaudeApiError` in `crates/mika-common/src/claude.rs` -- typed error enum with `is_retryable()` method
- `TelegramApiError` in `crates/mika-gateway/src/telegram.rs` -- enum with variant-based classification
- `forward_error_message(is_connect)` in `routes.rs` -- existing error classification pattern

**Test scenarios:**
- Happy path: ForwardResult classifies 200 and 202 as success
- Happy path: ForwardResult classifies 429 as retryable
- Happy path: ForwardResult classifies 500, 502, 503 as retryable
- Error path: ForwardResult classifies 400, 404 as permanent
- Error path: ForwardResult classifies connection errors (`is_connect()`) as permanent
- Edge case: ForwardResult classifies timeout errors as retryable

**Verification:**
- `forward_github_event` returns `ForwardResult` instead of `()`
- Existing spawn task in `handle_github_webhook` still compiles and behaves identically (log on non-success, no retry yet)
- All existing tests pass unchanged

- [x] **Unit 2: Implement deliver_with_retry wrapper**

**Goal:** Add the retry-with-backoff wrapper that calls `forward_github_event` up to 6 times (initial + 5 retries) with the specified delay schedule, releasing the semaphore during sleeps.

**Requirements:** R1, R2, R3, R4, R5, R6, R8

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-gateway/src/github.rs`
- Test: `crates/mika-gateway/src/github.rs` (inline tests)

**Approach:**
- Define `const RETRY_DELAYS: [Duration; 5]` with the schedule `[2s, 5s, 15s, 60s, 300s]`.
- Implement `deliver_with_retry` as an async function that takes the same arguments as `forward_github_event` plus a reference to the semaphore and the delay schedule.
- On the initial attempt, the caller's permit is used. On retryable failure, drop the permit, sleep, then `try_acquire_owned` before retrying. If the semaphore is full, log WARN and abandon (the system is overloaded).
- After exhausting retries, log ERROR with `delivery_id`, `target_agent`, attempt count, and last error description.
- Intermediate retry attempts log WARN with attempt number, delay, and reason.
- Replace the `tokio::spawn` body in `handle_github_webhook` to call `deliver_with_retry`.

**Execution note:** The retry wrapper does not need to be generic -- it is specific to GitHub webhook delivery. If Telegram or other channels need retry later, extract then.

**Patterns to follow:**
- `for attempt in 0..=MAX_RETRIES` pattern from `crates/mika-common/src/claude.rs` (lines 452-480)
- Structured logging with `delivery_id`, `target_agent`, `attempt` fields matching the existing tracing conventions in `github.rs`

**Test scenarios:**
- Happy path: first attempt succeeds, no retry, no extra logging
- Happy path: first attempt returns 429, second attempt succeeds after 2s delay -- verify only 2 calls made
- Happy path: first attempt returns 503, second attempt succeeds -- verify retry behavior
- Error path: 400 on first attempt -- no retry, WARN logged immediately
- Error path: connection error on first attempt -- no retry, ERROR logged
- Error path: all 6 attempts return 429 -- ERROR logged with delivery_id and target_agent after exhaustion
- Edge case: semaphore unavailable on retry -- WARN logged, retry abandoned
- Edge case: timeout on first attempt -- classified as retryable, retry occurs
- Integration: deliver_with_retry called from `handle_github_webhook` spawn task -- verify permit is held during attempt, released during sleep

**Verification:**
- `cargo test -p mika-gateway` passes
- `cargo clippy -p mika-gateway` clean
- The retry schedule matches `[2s, 5s, 15s, 60s, 300s]` exactly

- [x] **Unit 3: Add inline documentation for dedup LRU eviction edge case**

**Goal:** Document the known interaction between late retries and the LRU delivery cache for future maintainers.

**Requirements:** R7

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-gateway/src/github.rs`

**Approach:**
- Add a doc comment on `deliver_with_retry` explaining: the delivery_id is inserted into the LRU cache before forwarding. Under extreme volume (>10k deliveries during a single 300s retry sleep), the LRU may evict the entry. If GitHub redelivers the same event, the gateway would accept it as new. Agent-side idempotency (work item unique index) prevents duplicate processing. A TTL on the LRU cache would fix this but is deferred to a separate ticket.
- Add a brief comment in the spawn task noting the retry budget and cross-referencing #590 (DLQ).

**Test expectation:** none -- documentation only

**Verification:**
- Comments exist on `deliver_with_retry` and the spawn task explaining the dedup edge case and the DLQ cross-reference

## System-Wide Impact

- **Interaction graph:** The retry wrapper interacts with: (1) semaphore shared with Telegram webhooks -- releasing during sleep prevents starvation; (2) delivery LRU cache -- read-only during retry, known eviction edge case documented; (3) agent container's `/message` endpoint -- receives the same payload on each attempt with the same `request_id`.
- **Error propagation:** `forward_github_event` now returns `ForwardResult` to the retry wrapper. The wrapper decides whether to retry or log and stop. No error propagation beyond the spawned task (it's fire-and-forget from the handler's perspective).
- **State lifecycle risks:** Semaphore permits are released during sleep, creating a window where another task could use the permit. This is intentional -- it prevents starvation. The re-acquisition via `try_acquire_owned` handles the overload case gracefully.
- **API surface parity:** The Telegram forwarding path (`handle_forward_result` in `routes.rs`) does NOT need this change -- Telegram has built-in retry via `reset_dedup()`. The A2A proxy path does not use webhook forwarding.
- **Unchanged invariants:** The handler still returns 200 to GitHub immediately. The delivery cache dedup logic is unchanged. The semaphore capacity (30) is unchanged. The per-request 5s timeout on `forward_github_event` is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Semaphore contention increases under sustained 429s (permits released and re-acquired more frequently) | Natural backpressure: if semaphore is full on re-acquire, retry is abandoned. 30 permits is generous for the current webhook volume. |
| Late retry (300s) delivers duplicate if LRU evicted the delivery_id | Agent-side idempotency (work item unique index). Documented as known edge case. TTL on LRU deferred. |
| Gateway restart during retry loop loses the event | Acceptable for MVP. DLQ in #590 provides persistence. |
| Retry wrapper holds tokio task alive for up to ~382s | This is a lightweight async sleep, not a thread. Tokio handles thousands of sleeping tasks efficiently. |

## Sources & References

- Related issues: #589 (this), #590 (DLQ -- gated on this), #583/PR #586 (engine-side dispatch guards), #571/PR #587 (check_suite handler)
- Existing retry pattern: `crates/mika-common/src/claude.rs` lines 452-480
- Learnings: `docs/solutions/code-review-patterns/async-callbacks-long-running-review-findings.md` (P2-529 tight retry)
- Learnings: `docs/solutions/ux-improvements/gateway-offline-agent-error-message.md` (error classification)
