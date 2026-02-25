---
status: pending
priority: p2
issue_id: "214"
tags: [code-review, performance, skills-system]
dependencies: ["213"]
---

# reqwest::Client::new() Created Per HTTP Tool Call

## Problem Statement
`execute_http()` in `handler.rs` creates a new `reqwest::Client` for every tool call. This skips HTTP connection pooling, forces new TLS handshakes, and adds latency. Should be a shared client.

## Findings
- Location: `crates/mika-agent/src/skills/handler.rs:91`
- `let client = reqwest::Client::new();` inside the function body
- No connection reuse between calls
- If HTTP handler is kept (see #213), client should be shared

## Proposed Solutions

### Option 1: Remove with HTTP handler (#213)
- **Pros**: Issue disappears
- **Effort**: Small
- **Risk**: None

### Option 2: Pass shared client as parameter
- **Pros**: Proper connection pooling
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/handler.rs`

## Acceptance Criteria
- [ ] reqwest::Client shared across calls (or HTTP handler removed per #213)

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — performance-oracle agent
