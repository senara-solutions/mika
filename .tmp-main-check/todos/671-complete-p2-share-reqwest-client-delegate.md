---
status: pending
priority: p2
issue_id: "671"
tags: [code-review, performance, delegate-task]
dependencies: []
---

# Share reqwest::Client in Delegate Sender

## Problem Statement

`delegate_task.rs:202` creates a fresh `reqwest::Client::new()` per delegation instead of reusing the shared HTTP client. Each new client allocates a connection pool, TLS session cache, and DNS resolver. For sequential delegations in a team run, this means N TLS handshakes to the same gateway instead of 1.

## Findings

- Flagged by performance-oracle and pattern-recognition agents
- Server handlers at `handlers.rs:217` and `mod.rs:183` correctly pass shared `http_client.clone()`
- `DelegateTaskTool` struct has `settings` but no HTTP client reference

## Proposed Solutions

### Option A: Pass HTTP client through ToolContext (Recommended)
- Add `http_client: &reqwest::Client` to `ToolContext` or pass via `DelegateTaskTool` fields
- **Effort:** Small
- **Risk:** Low

### Option B: Create a lazy static client in DelegateTaskTool
- Use `OnceLock<reqwest::Client>` in the tool struct
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/delegate_task.rs:202`

## Acceptance Criteria

- [ ] Delegate sender reuses shared HTTP client
- [ ] Connection pooling works across sequential delegations
