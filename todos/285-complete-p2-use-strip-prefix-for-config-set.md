---
status: pending
priority: p2
issue_id: 285
tags: [code-review, quality, consistency]
dependencies: []
---

# Use strip_prefix("set") instead of args[3..] in handle_config

## Problem Statement

`handle_config` uses `args.starts_with("set")` + `args[3..]` byte slicing, which is fragile if the prefix changes. The codebase already uses `strip_prefix` elsewhere (e.g., `handle_memory` uses `args.strip_prefix("search")`).

## Findings

- **Architecture Strategist:** `args[3..]` vs `strip_prefix` inconsistency — fragile to refactoring

## Proposed Solutions

### Solution A: Use strip_prefix (Recommended)

**File:** `crates/mika-cli/src/tui/commands/handlers.rs:260-261`

```rust
if let Some(rest) = args.strip_prefix("set") {
    return handle_config_set(app, rest.trim()).await;
}
```

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] `strip_prefix("set")` used instead of `args[3..]`
- [ ] `/config set chat_id 123` still works
- [ ] `/config settings` does not trigger the set handler
