---
status: complete
priority: p3
issue_id: 607
tags: [code-review, ux, robustness]
dependencies: []
---

# Add TTY guard to `mika setup` for non-interactive contexts

## Problem Statement

The setup wizard uses `dialoguer` for interactive prompts but does not check whether stdin is a TTY. In non-TTY contexts (CI pipelines, subprocess invocations), `dialoguer` will fail with an opaque error. The skills installer in the same codebase already has this guard pattern (`std::io::stdin().is_terminal()` at `skills.rs:386`).

## Findings

- **Source:** pattern-recognition-specialist + agent-native-reviewer agents
- **Location:** `crates/mika-cli/src/commands/setup.rs` — top of `run()` function
- **Evidence:** `skills.rs:386` has TTY check; `setup.rs` does not
- **Note:** If all values are pre-set, prompts are skipped and the TTY guard is not needed. But partial config (some values set, some not) would still trigger prompts in a non-TTY context.

## Proposed Solutions

### Option A: Add TTY check with clear error message (Recommended)
```rust
use std::io::IsTerminal;
if !std::io::stdin().is_terminal() {
    // If everything is already set, proceed silently
    // Otherwise, bail with guidance
    anyhow::bail!(
        "mika setup requires an interactive terminal. \
         Pre-set MIKA_ANTHROPIC_API_KEY and other env vars, \
         or run `mika setup` in a terminal."
    );
}
```
- Effort: Small
- Risk: Low

### Option B: Skip prompts in non-TTY mode
Instead of bailing, silently skip all prompts in non-TTY mode (only run bootstrap + auto-generate token).
- Effort: Small
- Risk: Low — but may surprise users who expected to be prompted

## Acceptance Criteria

- [x] `mika setup` in a non-TTY context either succeeds silently or fails with a clear message
- [x] Consistent with TTY guard pattern in `skills.rs`
