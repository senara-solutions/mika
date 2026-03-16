---
status: pending
priority: p2
issue_id: "682"
tags: [code-review, reliability, ux]
dependencies: []
---

# Missing timeout on LLM call in soul generation

## Problem Statement

`generate_soul_md` in `wizard.rs` calls `provider.send_message()` with no timeout wrapper. If the LLM provider is slow or unresponsive, the user stares at "Generating personality..." indefinitely. The agent loop uses 30-second per-tool timeouts, but this creation path has none.

## Findings

- **Architecture Strategist**: Medium risk. Users on slow connections or misconfigured providers will experience a hang.
- **Security Sentinel**: Corroborated — no timeout protection.

**Affected files:**
- `crates/mika-cli/src/commands/agents.rs` (`try_generate_soul` function)
- `crates/mika-cli/src/wizard.rs` (`generate_soul_md` function)

## Proposed Solutions

### Option A: Wrap with tokio::time::timeout (Recommended)
In `try_generate_soul`, wrap the `generate_soul_md` call:
```rust
use tokio::time::{timeout, Duration};
match timeout(Duration::from_secs(30), wizard::generate_soul_md(...)).await {
    Ok(Some(soul)) => Some(soul),
    _ => { println!("  Could not generate personality, using template."); None }
}
```
- **Pros:** Simple, consistent with 30s tool timeout convention
- **Cons:** None
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] LLM call times out after 30 seconds with a user-friendly message
- [ ] Template fallback is used on timeout
