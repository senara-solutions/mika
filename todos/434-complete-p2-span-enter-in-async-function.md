---
status: complete
priority: p2
issue_id: "434"
tags: [code-review, correctness, tracing]
dependencies: []
---

# span.enter() in async function should use .instrument()

## Problem Statement

In `crates/mika-agent/src/agent.rs`, `run_team_agent_inner` uses `span.enter()` in an async function:

```rust
let span = tracing::info_span!("team_agent", agent = %params.agent_name);
let _guard = span.enter();
```

`span.enter()` creates a guard that is not `Send` and should not be held across `.await` points. The tracing docs recommend `Instrument::instrument()` for async functions. While this works in practice (the guard is held for the entire function on a single tokio task), it is technically incorrect and could produce misleading span data if the task migrates between threads.

## Findings

- Performance agent flagged this as a correctness concern
- The tracing docs explicitly recommend `.instrument(span)` for async contexts

## Proposed Solutions

### Option A: Use `Instrument` trait (Recommended)

Restructure `run_team_agent_inner` to use the `Instrument` trait:

```rust
use tracing::Instrument;

async fn run_team_agent_inner(params: &TeamAgentParams<'_>) -> Result<Option<String>> {
    run_team_agent_inner_impl(params)
        .instrument(tracing::info_span!("team_agent", agent = %params.agent_name))
        .await
}

async fn run_team_agent_inner_impl(params: &TeamAgentParams<'_>) -> Result<Option<String>> {
    // ... existing body
}
```

- **Pros:** Correct async tracing, follows tracing docs
- **Cons:** Adds a wrapper function
- **Effort:** Small
- **Risk:** None

## Technical Details

- **File:** `crates/mika-agent/src/agent.rs`, line ~1348
- **Components:** Team agent loop

## Acceptance Criteria

- [ ] `span.enter()` replaced with `.instrument()` pattern
- [ ] Agent name appears in log output for team agent runs
