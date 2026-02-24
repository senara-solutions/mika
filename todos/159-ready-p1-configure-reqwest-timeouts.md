---
status: ready
priority: p1
issue_id: "159"
tags: [code-review, security, performance]
---

# Configure reqwest::Client With Timeouts and Pool Limits

## Problem Statement
`reqwest::Client::new()` in main.rs:42 uses defaults with no timeouts. Outbound Telegram API calls have no timeout — a hung connection blocks the spawned task indefinitely. Container forwarding has per-request 2s timeout but no connect timeout.

## Findings
- **Security sentinel**: No timeout on Telegram API calls; hung connection = resource leak
- **Performance oracle**: No pool_max_idle_per_host; idle connections to inactive containers accumulate

## Proposed Solutions

### Option A: Configure client builder (Recommended)
```rust
let http_client = reqwest::Client::builder()
    .timeout(Duration::from_secs(10))
    .connect_timeout(Duration::from_secs(2))
    .pool_max_idle_per_host(10)
    .pool_idle_timeout(Duration::from_secs(90))
    .build()?;
```
- Pros: Bounded resource usage, faster failure detection
- Cons: None
- Effort: Small (5 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/main.rs` (line 42)

## Acceptance Criteria
- [ ] Global timeout set on reqwest client
- [ ] Connect timeout set
- [ ] Pool limits configured
- [ ] All existing tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
