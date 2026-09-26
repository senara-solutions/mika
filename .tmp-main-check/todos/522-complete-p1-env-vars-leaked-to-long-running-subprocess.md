---
status: complete
priority: p1
issue_id: 522
tags: [code-review, security, skills]
dependencies: []
---

# MIKA_* Environment Variables Leaked to Long-Running Subprocesses

## Problem Statement

`spawn_long_running_exec` in `crates/mika-agent/src/skills/executor.rs` spawns background subprocesses that inherit the full parent environment, only stripping `TMUX` and `TMUX_PANE`. This leaks `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`, `MIKA_OPENAI_API_KEY`, and other secrets to the subprocess.

Long-running processes use `kill_on_drop(false)` and can persist for up to 90 days (the timeout clamp maximum), maximizing the window for secret exfiltration. A marketplace-installed skill with `long_running: true` would have full access to all API keys.

CLAUDE.md documents "Exec handler executor scrubs all MIKA_* env vars from child processes (defense-in-depth)" but this is not implemented in the Rust executor — individual handler scripts do their own `unset`.

**Severity:** P1 — Secret exfiltration risk from marketplace skills.

## Findings

- `crates/mika-agent/src/skills/executor.rs` lines 530-539 — only `.env_remove("TMUX")` and `.env_remove("TMUX_PANE")`
- Regular exec path (`execute_exec`) has the same minimal env removal
- MCP client uses `env_clear()` + allowlist (the gold standard)
- `crates/mika-agent/src/skills/git.rs` scrubs MIKA_* vars explicitly

## Proposed Solutions

1. **Add MIKA_* env scrubbing to spawn_long_running_exec** (and execute_exec for consistency)
   - Iterate over `std::env::vars()` and `.env_remove()` any key starting with `MIKA_`
   - Pros: Simple, matches git.rs pattern
   - Cons: Still inherits all other env vars
   - Effort: Small
   - Risk: Low

2. **Use env_clear() + allowlist** matching MCP pattern
   - Pros: Most secure, defense-in-depth
   - Cons: May break skills that depend on other env vars (PATH, HOME, LANG, etc.)
   - Effort: Medium
   - Risk: Medium (breaking change for existing skills)

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/executor.rs`
- **Components:** Skills executor, subprocess spawning

## Acceptance Criteria

- [ ] `spawn_long_running_exec` scrubs all MIKA_* env vars before spawning
- [ ] `execute_exec` also scrubs MIKA_* env vars for consistency
- [ ] Test verifies MIKA_* vars are not present in child process environment
