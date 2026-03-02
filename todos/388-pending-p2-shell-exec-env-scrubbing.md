---
status: pending
priority: p2
issue_id: 388
tags: [code-review, security, shell-exec]
dependencies: []
---

# Add environment variable scrubbing to shell-exec handler

## Problem Statement

The `shell-exec/handlers/run.sh` handler runs `eval "$COMMAND"` with the full inherited environment, including `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`, `MIKA_OPENAI_API_KEY`, and `MIKA_BRAVE_API_KEY`. A command like `env` or `printenv MIKA_ANTHROPIC_API_KEY` would leak secrets.

The `github/handlers/run.sh` already scrubs these variables (line 15):
```sh
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY
```

The shell-exec handler should do the same for defense-in-depth.

## Findings

- **Source**: security-sentinel review of PR #47
- **Severity**: Medium (pre-existing, not introduced by PR #47)
- **Evidence**: Compare `github/handlers/run.sh:15` (scrubs env) vs `shell-exec/handlers/run.sh` (no scrubbing)
- **Mitigating factors**: Docker containers run as non-root user `mika` (UID 1000); shell-exec is excluded from autonomous heartbeat runs via `safe_always_on_skills()`

## Proposed Solutions

### Option A: Add `unset` to run.sh (Recommended)
Add `unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY` at the top of `run.sh`, matching the github handler pattern.

- **Pros**: Simple, consistent, defense-in-depth
- **Cons**: Determined attacker could still read `/proc/self/environ` or re-read configs
- **Effort**: Small
- **Risk**: None

### Option B: Apply env_clear() in executor.rs
Apply the same `env_clear()` + allowlist pattern used for MCP child processes in the Rust executor at `executor.rs:246`.

- **Pros**: Stronger protection (no env inheritance at all), consistent with MCP pattern
- **Cons**: Medium effort, may break commands that depend on env vars (PATH, HOME, etc.)
- **Effort**: Medium
- **Risk**: Low (but needs testing for PATH/HOME requirements)

## Technical Details

- **Affected files**: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`
- **Reference pattern**: `crates/mika-agent/templates/skills/github/handlers/run.sh:15`

## Acceptance Criteria

- [ ] `MIKA_*` environment variables are not accessible from commands run via `run_shell`
- [ ] Existing tests pass
- [ ] Commands still have access to PATH, HOME, USER and other standard env vars
