---
status: complete
priority: p2
issue_id: "206"
tags: [code-review, ux, security]
dependencies: []
---

# $EDITOR Multi-Word Support in Config Edit

## Problem Statement

`Command::new(&editor)` treats the entire `$EDITOR` value as a binary path. Multi-word editors like `code --wait` or `emacs -nw` fail silently because the space-containing string is treated as a single executable name.

## Findings

- **Source:** security-sentinel (Finding 1)
- **Location:** `crates/mika-cli/src/commands/config.rs:23-25`
- **Evidence:** `Command::new(&editor).arg(&identity_path).status()?` — does not split on whitespace
- **Impact:** Users with `EDITOR="code --wait"` or `EDITOR="emacs -nw"` cannot use `mika config edit`

## Proposed Solutions

### Option 1: Split $EDITOR on whitespace
- **Pros**: Supports multi-word editors; preserves Command::new safety (no shell)
- **Cons**: None
- **Effort**: Trivial
- **Risk**: Low

```rust
let parts: Vec<&str> = editor.split_whitespace().collect();
let status = Command::new(parts[0]).args(&parts[1..]).arg(&identity_path).status()?;
```

## Recommended Action

Option 1.

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/config.rs`

## Acceptance Criteria

- [ ] `EDITOR="code --wait" mika config edit` works
- [ ] `EDITOR="vi" mika config edit` still works
- [ ] No shell injection possible (Command::new is not sh -c)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
