---
status: pending
priority: p3
issue_id: "686"
tags: [code-review, quality]
dependencies: ["685"]
---

# Simplify redundant match arm in try_generate_soul

## Problem Statement

`try_generate_soul` in agents.rs contains a no-op match arm `Some(soul) => Some(soul)`. This can be simplified to an `if result.is_none()` pattern.

## Findings

- **Code Simplicity Reviewer**: Replace the match with a simpler pattern.

**Affected files:**
- `crates/mika-cli/src/commands/agents.rs` (`try_generate_soul` function)

## Proposed Solutions

Replace:
```rust
match wizard::generate_soul_md(...).await {
    Some(soul) => Some(soul),
    None => { println!("..."); None }
}
```
With:
```rust
let result = wizard::generate_soul_md(...).await;
if result.is_none() {
    println!("  Could not generate personality, using template.");
}
result
```

- **Effort:** Small
- **Risk:** Low
