---
status: pending
priority: p2
issue_id: 273
tags: [code-review, agent-native, ux]
dependencies: []
---

# Handle empty response in mika ask subcommand

## Problem Statement

`mika ask "message"` prints a blank line to stdout when the agent returns an empty string (tool-only response). This is the same class of bug fixed in the TUI but not addressed for the non-interactive CLI path.

## Findings

- **File**: `crates/mika-cli/src/commands/ask.rs:49`
- **Impact**: Medium — confusing for scripts and users
- **Found by**: agent-native-reviewer

## Proposed Solution

```rust
if response.is_empty() {
    eprintln!("(Agent processed your request — no text response)");
} else {
    println!("{response}");
}
```

## Acceptance Criteria

- [ ] Empty response prints diagnostic to stderr
- [ ] Non-empty response prints to stdout (unchanged)
