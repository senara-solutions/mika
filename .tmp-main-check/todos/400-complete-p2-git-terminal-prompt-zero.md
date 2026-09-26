---
status: complete
priority: p2
issue_id: "400"
tags: [code-review, security, marketplace, pr-56]
dependencies: []
---

# Missing GIT_TERMINAL_PROMPT=0 on git subprocesses

## Problem Statement

`git_command()` does not set `GIT_TERMINAL_PROMPT=0`. If a user provides a URL to a private repository, git will prompt for credentials on the terminal, causing the CLI to hang indefinitely.

## Findings

- **Source**: security-sentinel
- **File**: `crates/mika-agent/src/skills/git.rs:111-122`

## Proposed Solutions

### Option A: Set GIT_TERMINAL_PROMPT=0 (Recommended)

```rust
fn git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    // ... existing MIKA_* scrubbing
    cmd
}
```

- Effort: Small (1 line)
- Risk: Low

## Acceptance Criteria

- [ ] `git_command()` sets `GIT_TERMINAL_PROMPT=0`
- [ ] Git operations fail cleanly on private repos instead of hanging

## Resources

- `crates/mika-agent/src/skills/git.rs:111-122`
