# Plan: bug(gateway): webhook deliveries during mika-server restart get permanent-no-retry

**Ticket:** mika issue#1293
**Type:** bug fix
**Scope:** `crates/mika-gateway/src/github.rs`

## Problem

When mika-server restarts (e.g., during `make deploy`), in-flight webhook deliveries from `mika-gateway` fail with a connection error. The error classification at line 953 of `github.rs` treats `e.is_connect()` as `ForwardResult::Permanent`, which:

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

### Step 1: Reclassify connection errors as retryable

**File:** `crates/mika-gateway/src/github.rs`

Change the `e.is_connect()` branch in `forward_to_resolved_route()` from `Permanent` to `Retryable`:

```rust
Err(e) => {
    if e.is_connect() {
        // Connection refused — agent may be restarting. Retry with backoff;
        // if all retries fail the event falls through to DLQ persistence.
        ForwardResult::Retryable {
            reason: format!("connection error: {e}"),
        }
    } else {
        // Timeout or other transient network error — worth retrying.
        ForwardResult::Retryable {
            reason: format!("network error: {e}"),
        }
    }
}
```

This is the entire code change. Both branches now return `Retryable`, which means:

- The retry loop applies the existing `[2s, 5s, 15s, 60s, 300s]` schedule (6 attempts total, 382s worst-case)
- If mika-server comes back within that window (typical restart is 30-60s), the event delivers on retry 2-4
- If all retries fail, the event is persisted to the DLQ (line 1163) instead of being dropped
- The DLQ background worker (30s tick, exponential backoff up to 1h, 10 max attempts) provides a safety net

### Step 2: Update `ForwardResult` doc comment

Update the `Permanent` variant doc comment to remove "connection error" from the list since it's no longer classified that way:

```rust
/// 4xx (other than 429) or unresolvable route — retrying will not help.
Permanent {
    /// Human-readable description for logging.
    reason: String,
},
```

And update the `Retryable` variant doc:

```rust
/// 429 or 5xx, request timeout, or connection error (agent may be restarting) —
/// transient, worth retrying.
Retryable {
    /// Human-readable description for logging (e.g. "HTTP 429" or "connection error").
    reason: String,
},
```

### Step 3: Update `deliver_with_retry` doc comment

The doc comment on `deliver_with_retry` (line 968-996) describes permanent failures including "connection errors indicating the agent is offline." Update to reflect the new classification:

```
/// On a retryable failure (429, 5xx, request timeout, or connection error),
/// releases the permit, sleeps for the next delay...
```

### Step 4: Update CLAUDE.md

In `crates/mika-gateway/CLAUDE.md`, the "Inbound delivery retry (#589)" section says:

> Permanent failures (HTTP 4xx other than 429, connection errors indicating the agent is offline, or unresolvable route) stop retries immediately.

Update to:

> Permanent failures (HTTP 4xx other than 429, or unresolvable route) stop retries immediately. Connection errors are retryable (#1293) — the agent may be restarting during a deploy.

### Step 5: Update existing tests and add new test

Check existing tests in `github.rs` that assert connection errors are `Permanent` and update them to assert `Retryable`. Add a test that verifies connection errors trigger the retry path:

```rust
#[test]
fn test_connection_error_is_retryable() {
    // Verify that connection errors are classified as Retryable, not Permanent,
    // so that deploy-restart windows are covered by the retry schedule (#1293).
    // (Exact test shape depends on existing test patterns in the file.)
}
```

## What This Does NOT Change

- **HTTP 4xx (except 429) remain Permanent.** A 400 Bad Request or 403 Forbidden from the agent is a real rejection, not a restart artifact.
- **Unresolvable routes remain Permanent.** No Postgres mapping + no fallback = no point retrying.
- **The retry schedule is unchanged.** `[2s, 5s, 15s, 60s, 300s]` already covers the restart window.
- **DLQ infrastructure is unchanged.** It already handles retryable-exhausted events correctly.
- **Semaphore lifecycle is unchanged.** Connection errors during retry still release and re-acquire permits normally.

## Risk Assessment

**Low risk.** This is a one-line classification change (Permanent → Retryable) that aligns connection errors with the existing retry + DLQ infrastructure. The worst case is that a genuinely-offline agent gets 6 retry attempts (382s) before hitting the DLQ instead of failing immediately — this is strictly better than dropping the event silently.

## Verification

1. `cargo test -p mika-gateway` — all existing tests pass with updated assertions
2. `cargo clippy -p mika-gateway` — no warnings
3. Manual: trigger a webhook during `make deploy` — event should deliver after mika-server restart completes (currently fails with permanent-no-retry)
