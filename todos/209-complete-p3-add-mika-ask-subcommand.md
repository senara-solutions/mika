---
status: complete
priority: p3
issue_id: "209"
tags: [code-review, agent-native, feature]
dependencies: []
---

# Add `mika ask` Non-Interactive Subcommand

## Problem Statement

The old CLI used stdin/stdout REPL which could be piped (`echo "hello" | mika-cli`). The new TUI enters full alternate screen with raw mode, making the CLI non-scriptable. Automated processes and scripts lose the ability to send a message and read a response through the CLI path.

## Findings

- **Source:** agent-native-reviewer (Finding 3)
- **Location:** `crates/mika-cli/src/commands/chat.rs` — full TUI takeover
- **Evidence:** `terminal::enable_raw_mode()`, `EnterAlternateScreen` — incompatible with piped input
- **Impact:** Scripts, CI jobs, and other agents cannot compose with the CLI. The HTTP server provides an alternative, but the CLI path is no longer scriptable.

## Proposed Solutions

### Option 1: Add `mika ask "<message>"` subcommand
- **Pros**: Restores scriptability; simple stdin → agent → stdout flow; composable
- **Cons**: New subcommand
- **Effort**: Small (reuse init + run_agent, print response to stdout)
- **Risk**: Low

```
mika ask "What's on my calendar tomorrow?"
# prints response to stdout, exits 0
```

## Recommended Action

Option 1 — small addition that makes the CLI agent-native (composable by other programs).

## Technical Details

- **Affected files:** `crates/mika-cli/src/cli.rs` (new variant), `crates/mika-cli/src/commands/` (new ask.rs)

## Acceptance Criteria

- [ ] `mika ask "hello"` prints agent response to stdout and exits
- [ ] Works with piped input: `echo "hello" | mika ask -`
- [ ] Exit code 0 on success, non-zero on error

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | Agent-native parity requires scriptable paths |

## Resources

- Commit: 399ebf0
