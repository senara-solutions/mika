---
status: pending
priority: p3
issue_id: "687"
tags: [code-review, security, validation]
dependencies: []
---

# No input length validation on wizard inputs

## Problem Statement

The dialoguer `Input` prompts in `wizard.rs` accept arbitrary-length strings for `display_name`, `emoji`, `specialization`, and `communication_style`. Very long inputs could create oversized files or LLM requests that exceed token limits.

## Findings

- **Security Sentinel**: Low severity. Local CLI, user is the operator. But reasonable length limits would be good hygiene.

**Affected files:**
- `crates/mika-cli/src/wizard.rs` (all `Input::new()` calls)

## Proposed Solutions

Add `.validate_with()` to cap lengths (e.g., 200 chars for display_name, 500 for specialization):
```rust
Input::new()
    .with_prompt("  Display name")
    .default(default_display)
    .validate_with(|input: &String| {
        if input.len() > 200 { Err("Display name must be under 200 characters") }
        else { Ok(()) }
    })
    .interact_text()?;
```

- **Effort:** Small
- **Risk:** Low
