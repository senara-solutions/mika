# Plan: bug(gateway): webhook deliveries during mika-spirit restart get permanent-no-retry

**Ticket:** mika issue#1293
**Type:** bug fix
**Scope:** `crates/mika-gateway/src/github.rs`

## Problem

When mika-spirit restarts (e.g., during `make deploy`), in-flight webhook deliveries from `mika-gateway` fail with a connection error. The error classification at line 953 of `github.rs` treats `e.is_connect()` as `ForwardResult::Permanent`, which:

1. Stops retries immediately (the retry loop only retries `Retryable` results)
2. Returns without persisting to the DLQ (only `Retryable`-exhausted events reach DLQ insertion at line 1163)
3. The event is silently dropped — GitHub already received 200 OK (async delivery), so GitHub won't redeliver

The comment on line 954 says "Connection refused / DNS failure — agent is offline, retrying won't help." This was correct for the original design assumption (agent permanently offline), but incorrect for the deploy-restart case where the agent comes back within 30-60 seconds.

## Root Cause

```rust
// github.rs line 952-957
Err(e) => {
    if e.is_connect() {
        // Connection refused / DNS failure — agent is offline, retrying won't help.
        ForwardResult::Permanent {
            reason: format!("connection error: {e}"),
        }
    }
}
```

Connection errors are classified as permanent, but during a restart they are transient. The existing retry schedule `[2s, 5s, 15s, 60s, 300s]` (382s total worst-case) already covers the restart window.

## Fix

### Step 1: Reclassify connection errors as retryable for localhost routes

**File:** `crates/mika-gateway/src/github.rs`

Change the `e.is_connect()` branch in `forward_to_resolved_route()` from `Permanent` to `Retryable`, **scoped to localhost container URLs only**. The `route.container_url` is available in scope (line 920). Connection errors to non-localhost routes (e.g., external webhook receivers or remote agent URLs) remain `Permanent` — the issue body "Out of scope" section explicitly defers generalization: *"Start with the specific `localhost:8081/message` route; extend if other routes hit the same pattern."*

The only line changing classification is the `e.is_connect()` branch (currently `Permanent`). The `else` branch already returns `Retryable` and is unchanged — shown here for context only.

```rust
Err(e) => {
    if e.is_connect() {
        // Connection refused to a local agent — may be restarting during deploy.
        // Retry with backoff; if all retries fail the event falls through to DLQ.
        // Scoped to localhost per mika#1293 — extend to other routes if the
        // same pattern is observed (see issue "Out of scope").
        let is_localhost = route.container_url.starts_with("http://localhost")
            || route.container_url.starts_with("http://127.0.0.1");
        if is_localhost {
            ForwardResult::Retryable {
                reason: format!("connection error (localhost, retryable): {e}"),
            }
        } else {
            ForwardResult::Permanent {
                reason: format!("connection error: {e}"),
            }
        }
    } else {
        // Timeout or other transient network error — already retryable (unchanged).
        ForwardResult::Retryable {
            reason: format!("network error: {e}"),
        }
    }
}
```

The net change is: `e.is_connect()` on localhost routes flips from `Permanent` to `Retryable`. Non-localhost connection errors and the `else` branch are unchanged. This means:

- For localhost connection errors, the retry loop applies the existing `[2s, 5s, 15s, 60s, 300s]` schedule (6 attempts total, 382s worst-case)
- If mika-spirit comes back within that window (typical restart is 30-60s), the event delivers on retry 2-4
- If all retries fail, the event is persisted to the DLQ (line 1163) instead of being dropped
- The DLQ background worker (30s tick, exponential backoff up to 1h, 10 max attempts) provides a safety net
- Non-localhost connection errors remain `Permanent` — identical to current behavior

### Step 2: Update `ForwardResult` doc comment

Update the `Permanent` variant doc comment to clarify that connection errors are now conditionally retryable:

```rust
/// 4xx (other than 429), non-localhost connection error, or unresolvable route —
/// retrying will not help.
Permanent {
    /// Human-readable description for logging.
    reason: String,
},
```

And update the `Retryable` variant doc:

```rust
/// 429 or 5xx, request timeout, or localhost connection error (agent may be
/// restarting during deploy, #1293) — transient, worth retrying.
Retryable {
    /// Human-readable description for logging (e.g. "HTTP 429" or "connection error").
    reason: String,
},
```

### Step 3: Update `deliver_with_retry` doc comment

The doc comment on `deliver_with_retry` (line 968-996) describes permanent failures including "connection errors indicating the agent is offline." Update to reflect the new classification:

```
/// On a retryable failure (429, 5xx, request timeout, or localhost connection
/// error — see #1293), releases the permit, sleeps for the next delay...
```

### Step 4: Update CLAUDE.md

In `crates/mika-gateway/CLAUDE.md`, the "Inbound delivery retry (#589)" section says:

> Permanent failures (HTTP 4xx other than 429, connection errors indicating the agent is offline, or unresolvable route) stop retries immediately.

Update to:

> Permanent failures (HTTP 4xx other than 429, non-localhost connection errors, or unresolvable route) stop retries immediately. Localhost connection errors are retryable (#1293) — the agent may be restarting during a deploy.

### Step 5: Update existing tests and add new test

Check existing tests in `github.rs` that assert connection errors are `Permanent` and update them to assert `Retryable` (for localhost routes). Add a test that verifies the classification invariant:

```rust
#[tokio::test]
async fn test_localhost_connection_error_is_retryable() {
    // Simulate a connection error against a localhost container URL.
    // The exact mock/setup follows existing test patterns in the file
    // (e.g., mock HTTP client or a port that refuses connections).
    //
    // Primary assertion — the fix's correctness invariant (#1293):
    //   assert!(matches!(result, ForwardResult::Retryable { .. }));
    //
    // This locks in the classification so a future refactor cannot
    // silently re-introduce Permanent for localhost connection errors.
}

#[tokio::test]
async fn test_non_localhost_connection_error_remains_permanent() {
    // Simulate a connection error against a non-localhost container URL
    // (e.g., http://remote-host:8081). Verify the original Permanent
    // classification is preserved for non-localhost routes.
    //
    // Primary assertion:
    //   assert!(matches!(result, ForwardResult::Permanent { .. }));
}
```

Both tests assert the `ForwardResult` variant directly via `matches!()`. The first test prevents regression of the #1293 fix; the second ensures the localhost scoping (per issue "Out of scope" — review-guide.md § YAGNI) is not accidentally widened by a future change.

## What This Does NOT Change

- **HTTP 4xx (except 429) remain Permanent.** A 400 Bad Request or 403 Forbidden from the agent is a real rejection, not a restart artifact.
- **Non-localhost connection errors remain Permanent.** Connection errors to external/remote agent routes are not reclassified — per issue "Out of scope": *"Start with the specific `localhost:8081/message` route; extend if other routes hit the same pattern."* (review-guide.md § YAGNI)
- **Unresolvable routes remain Permanent.** No Postgres mapping + no fallback = no point retrying.
- **The retry schedule is unchanged.** `[2s, 5s, 15s, 60s, 300s]` already covers the restart window.
- **DLQ infrastructure is unchanged.** It already handles retryable-exhausted events correctly.
- **Semaphore lifecycle is unchanged.** Connection errors during retry still release and re-acquire permits normally.

## Risk Assessment

**Low risk.** This is a scoped classification change (Permanent → Retryable for localhost connection errors only) that aligns the deploy-restart case with the existing retry + DLQ infrastructure. The worst case is that a genuinely-offline localhost agent gets 6 retry attempts (382s) before hitting the DLQ instead of failing immediately — this is strictly better than dropping the event silently. Non-localhost routes are unaffected.

## Verification

1. `cargo test -p mika-gateway` — all existing tests pass with updated assertions
2. `cargo clippy -p mika-gateway` — no warnings
3. Manual: trigger a webhook during `make deploy` — event should deliver after mika-spirit restart completes (currently fails with permanent-no-retry)

## Revision history

- rev 2 (2026-05-26): addressed F1 by scoping the `e.is_connect()` reclassification to localhost container URLs only (`route.container_url.starts_with("http://localhost") || ...starts_with("http://127.0.0.1")`), preserving `Permanent` for non-localhost routes per issue "Out of scope" (review-guide.md § YAGNI); addressed F2 by specifying concrete assertion targets (`matches!(result, ForwardResult::Retryable { .. })` and `matches!(result, ForwardResult::Permanent { .. })`) in the test stubs and adding a second test for the non-localhost permanent-classification invariant.
