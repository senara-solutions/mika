---
status: pending
priority: p2
issue_id: "685"
tags: [code-review, observability]
dependencies: []
---

# Silent error swallowing in generate_soul_md

## Problem Statement

`generate_soul_md` in `wizard.rs` line 192 discards LLM errors with `Err(_) => None`. The caller prints "Could not generate personality, using template." but the actual error reason (API key invalid, network timeout, rate limit, etc.) is lost. This makes debugging LLM connectivity issues impossible.

## Findings

- **Security Sentinel**, **Architecture Strategist**, **Pattern Recognition**: All flagged this independently. Setup.rs maps errors explicitly; this function silently discards them.

**Affected files:**
- `crates/mika-cli/src/wizard.rs` (line 192)

## Proposed Solutions

Replace `Err(_) => None` with:
```rust
Err(e) => {
    tracing::debug!("soul.md generation failed: {e}");
    None
}
```

- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] LLM errors are logged at debug level
- [ ] User-facing output unchanged ("Could not generate personality, using template.")
