---
status: complete
priority: p2
issue_id: 290
tags: [code-review, performance, pattern-consistency]
dependencies: []
---

# CLI `make_message_sender` creates new reqwest::Client instead of sharing

## Problem Statement

`make_message_sender` in `chat.rs:50` creates a new `reqwest::Client::new()` on every call. The server mode already solved this — it creates one client in `run_server` and reuses it via `AppState.http_client`. Each CLI agent switch creates a new connection pool, wasting ~50-150ms for TLS renegotiation. This contradicts the pattern established by resolved todo #121.

## Findings

- **Performance Oracle:** Each `reqwest::Client::new()` bootstraps a hyper connection pool, DNS resolver, and TLS session cache. Called once per agent switch.
- **Pattern Recognition:** Server creates client once at `server/mod.rs:118` and passes `http_client.clone()` everywhere. CLI diverges from this pattern.
- **Architecture Strategist:** Minor inconsistency — functionally correct but doesn't follow server's client-reuse pattern.

## Proposed Solutions

### Solution A: Pass shared client through spawn_agent_worker

Create `reqwest::Client` once in `chat::run()` and pass it through.

```rust
// In chat::run(), before the loop:
let http_client = reqwest::Client::new();

// Pass to make_message_sender:
fn make_message_sender(settings: &Settings, db: &AsyncDatabase, http_client: &reqwest::Client) -> Option<Arc<dyn MessageSender>> {
    // ... use http_client.clone() instead of reqwest::Client::new()
}
```

- **Pros:** Matches server pattern exactly, enables TCP connection reuse across agent switches
- **Cons:** Adds one parameter to `make_message_sender` and `spawn_agent_worker`
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/chat.rs`
- **Related:** Todo #121 (completed) fixed the same issue in server mode

## Acceptance Criteria

- [ ] `reqwest::Client` created once per CLI session, not per agent worker
- [ ] Client passed through to `make_message_sender`
- [ ] Agent switches reuse the same connection pool
- [ ] All existing tests pass
