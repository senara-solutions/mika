---
module: gateway
date: 2026-04-16
problem_type: integration_issue
component: tooling
severity: high
symptoms:
  - "Gateway forwards GitHub webhook to agent, agent returns HTTP 429 (busy), gateway logs WARN and silently drops the event"
  - "PR sits APPROVED + green CI + mergeable but never auto-merges because the pull_request_review.submitted verdict was lost"
  - "GitHub already received 200 OK from the gateway, so it does not redeliver — the event is permanently gone"
root_cause: missing_workflow_step
resolution_type: code_fix
tags:
  - gateway
  - webhook
  - github
  - retry
  - resilience
  - backoff
related_components:
  - mika-gateway/github
  - mika-agent/server/handlers
---

# Gateway silently drops inbound webhooks when agent returns 429/5xx

## Problem

The mika-gateway receives a GitHub webhook, HMAC-validates it, routes it to the correct customer container, returns 200 OK to GitHub, and then `tokio::spawn`s a task to forward the event to the agent container via `POST /message`. If that forward gets HTTP 429 (agent busy) or 5xx (transient), the task logs a WARN and returns — no retry.

Because the gateway already returned 200 to GitHub at the entry point, GitHub does not retry either. The webhook is lost.

## Symptoms

**2026-04-15 22:35:00 UTC** — `pull_request_review.submitted` for mika#588 (mika-qa's approving verdict):

```
22:35:00 INFO  GitHub webhook routing event to agent
              event_type=pull_request_review action=submitted
              target_agent=mika-dev delivery_id=3c7b98c2-391b-11f1-...
22:35:00 WARN  agent container returned error for GitHub event
              status=429 target_agent=mika-dev request_id=3c7b98c2-...
```

mika-dev returned 429 because she was busy processing a claude-pilot callback for mika#579 (well-behaved backpressure). The gateway logged WARN and gave up. PR #588 sat APPROVED with green CI but was never auto-merged — the verdict handler never ran.

## What Didn't Work

Relying on GitHub's retry mechanism. The gateway returns 200 OK to GitHub the instant HMAC validates, before the spawned forwarding task even starts. GitHub treats the event as delivered — it does not retry on the basis of gateway-internal failures.

Relying on agent-side deferral queues. mika-agent has a 60s webhook deferral queue for callback sequencing, but it only activates for events that *arrive* at the agent. An event that returns 429 at the agent's HTTP handler never enters the queue.

## Solution

Add retry-with-backoff in the spawned forwarding task. Keep the single-attempt forwarding function pure; put the retry policy in a wrapper.

### Retry schedule

Fixed delays `[2s, 5s, 15s, 60s, 300s]` (initial attempt + 5 retries). Each delay gets ±25% jitter to prevent synchronized retry bursts when many events hit the same 429 simultaneously.

### What is retryable

- HTTP 429 (rate-limited / busy)
- HTTP 5xx (transient server error)
- Request timeouts (`reqwest::Error` that is not `is_connect()`) — transient overload

### What is NOT retryable

- HTTP 4xx other than 429 — client error, retry won't help
- Connection errors (`e.is_connect()`) — agent is offline, retry won't help
- Unresolvable routes (repo not registered + no fallback) — permanent config error

### Semaphore lifecycle

The 30-permit `webhook_semaphore` is shared between Telegram and GitHub forwarders. A naive implementation that holds the permit for the entire ~382s retry window would starve Telegram during sustained failures. Instead:

- Hold the caller's permit during the initial attempt
- Release during every retry sleep
- Re-acquire via `try_acquire_owned` before each retry attempt
- If the semaphore is full on re-acquire, abandon the retry with a **distinct** ERROR log (`semaphore at capacity during retry`) — do not fall through to the `retry budget exhausted` log, which would misrepresent attempt count and cause

### Route caching

`resolve_github_container_url` (Postgres query) runs once before the retry loop. The resolved `ResolvedRoute` is passed into every attempt. Without this, a single failing event hit the DB up to 6 times.

### Code shape

```rust
// crates/mika-gateway/src/github.rs

pub(crate) const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(300),
];

enum ForwardResult {
    Success,
    Retryable { reason: String },  // 429, 5xx, timeout
    Permanent { reason: String },  // 4xx, connection error
}

async fn deliver_with_retry(
    state: &AppState,
    target_agent: &str,
    text: &str,
    request_id: &str,
    repo_full_name: Option<&str>,
    initial_permit: OwnedSemaphorePermit,
    semaphore: &Arc<Semaphore>,
) {
    // Resolve route ONCE, cached across all retries.
    let route = match resolve_github_container_url(state, repo_full_name).await {
        Some(r) => r,
        None => { /* log and return */ return; }
    };

    let mut attempts_made = 0;
    let mut last_reason = String::new();
    let mut current_permit = Some(initial_permit);

    for (retry_idx, delay) in std::iter::once(None)
        .chain(RETRY_DELAYS.iter().map(Some))
        .enumerate()
    {
        if let Some(delay) = delay {
            tokio::time::sleep(apply_jitter(*delay)).await;
            match semaphore.clone().try_acquire_owned() {
                Ok(p) => current_permit = Some(p),
                Err(_) => {
                    error!(/* distinct "semaphore at capacity" log */);
                    return;
                }
            }
        }

        let result = forward_to_resolved_route(...).await;
        attempts_made = retry_idx + 1;
        current_permit.take();  // release before next sleep

        match result {
            ForwardResult::Success => return,
            ForwardResult::Permanent { .. } => return,
            ForwardResult::Retryable { reason } => { last_reason = reason; }
        }
    }

    error!(total_attempts = attempts_made, last_reason, "retry budget exhausted");
}
```

## Why This Works

**Classification at the attempt boundary.** `ForwardResult` turns the inner `reqwest` result into three categories the retry loop can match on without re-parsing HTTP status codes or error types. This keeps the single-attempt function pure and makes the retry decision explicit.

**Permit release prevents cross-channel starvation.** The semaphore throttles in-flight HTTP calls, not sleeping tasks. Releasing during sleep means a 10/sec webhook burst during a 6-min agent outage holds ~60 live tokio tasks at most but only 30 concurrent HTTP calls at any instant — Telegram traffic still flows.

**Distinct error logs preserve observability.** An event dropped because "we exhausted 6 attempts" has different operational meaning than one dropped because "the gateway ran out of permits". Conflating them (fall-through to a single ERROR) misleads incident response with wrong `total_attempts` and wrong `last_reason`.

**Jitter prevents self-reinforcing bursts.** Without jitter, 30 tasks that all got 429 at T=0 all wake at T=2s and re-hit the agent simultaneously — recreating the exact overload condition that caused the original 429. ±25% jitter spreads the retry window.

**Route caching bounds DB load.** Under sustained agent failure, retry amplifies Postgres load. Resolving once removes that amplification without changing correctness (the `github_repos` row is unlikely to change between retries seconds apart).

## Prevention

**Test the no-retry contract explicitly.** Tests for "no retry on 400" must assert `call_count == 1`, not just `available_permits == 30`. A regression that adds retries on 4xx would silently pass the permit assertion because the mock server falls back to 200.

```rust
assert_eq!(
    call_count.load(Ordering::SeqCst),
    1,
    "400 is permanent — must make exactly 1 HTTP call, no retry"
);
```

**Mark long-wall-clock tests `#[ignore]`.** The retry budget exhaustion test takes 2+5+15+60+300 = 382 seconds because `RETRY_DELAYS` is a compile-time const. Mark it `#[ignore]` and document how to run it:

```rust
#[tokio::test]
#[ignore = "takes ~6.5 minutes of wall time; run with --ignored"]
async fn test_deliver_retry_budget_exhausted_after_six_attempts() { ... }
```

**Use `tokio::time::timeout` to bound test wall time.** Any test that spawns `deliver_with_retry` and depends on timing should wrap `handle.await` in `tokio::time::timeout` so a race (semaphore blocker grabbed too late, etc.) produces a clean failure instead of a 5+ minute silent hang.

**When adding new classifications to the outcome enum, add tests on both sides.** Pure data-constructor tests (build the enum and check `is_retryable()`) prove the helper works. They do NOT prove that the single-attempt function produces the right variant. Add integration tests that drive real HTTP responses through the actual classification code.

**Document the dedup LRU interaction.** The gateway's 10k-entry LRU cache has no TTL. Under extreme webhook volume during a 300s retry sleep, an entry can be evicted, allowing GitHub redelivery to bypass dedup. Agent-side idempotency (task unique index) mitigates double-processing, but this is a known edge case — future work should add TTL (#590).

## References

- Issue: senara-solutions/mika#589
- Follow-up (DLQ): senara-solutions/mika#590
- Plan: `docs/plans/2026-04-16-003-feat-gateway-retry-inbound-webhook-delivery-plan.md`
- Related:
  - `docs/solutions/code-review-patterns/async-callbacks-long-running-review-findings.md` (P2-529 tight retry loop anti-pattern — avoided here)
  - `docs/solutions/ux-improvements/gateway-offline-agent-error-message.md` (error classification via `is_connect()`)
  - `docs/solutions/architecture-patterns/webhook-deferral-queue-callback-sequencing.md` (agent-side 60s queue — orthogonal to this retry)
  - `docs/solutions/integration-issues/gateway-pr-closed-webhook-routing.md` (prior silent-drop incident in the same handler)
