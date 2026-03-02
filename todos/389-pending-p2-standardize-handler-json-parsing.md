---
status: pending
priority: p2
issue_id: 389
tags: [code-review, architecture, consistency]
dependencies: []
---

# Standardize JSON parsing across all handler scripts

## Problem Statement

Handler scripts use three inconsistent JSON parsing approaches:
1. `jq`-only (no fallback): `github/run.sh`, `file-reader/read.sh`
2. `jq` + grep fallback: `shell-exec/run.sh` (after PR #47), `tmux/create_session.sh`
3. grep-only fallback (no `jq` check): `tmux/send_command.sh`, `tmux/read_output.sh`, `tmux/kill_session.sh`, `tmux/wait_for_text.sh`

The grep fallback is known-broken for any JSON string field containing double quotes. Since `jq` is already a hard dependency for `github` and `file-reader` handlers, and is installed in `Dockerfile.agent`, the inconsistency provides no safety benefit.

## Findings

- **Source**: agent-native-reviewer and code-simplicity-reviewer, PR #47
- **Severity**: Medium (consistency debt, latent bugs in tmux grep fallbacks)
- **Evidence**: `tmux/send_command.sh:19-22` uses grep-only for `TEXT` and `SPECIAL_KEY` fields — text containing quotes sent via tmux will be silently truncated if jq is absent

## Proposed Solutions

### Option A: Require jq everywhere, drop grep fallbacks (Recommended)
Make `jq` a hard dependency for all handlers. Remove grep fallbacks. Add a startup warning if `jq` is not found.

- **Pros**: Simplest, most consistent, eliminates all known-broken fallback paths
- **Cons**: Breaks on systems without jq (but those systems already fail on github/file-reader handlers)
- **Effort**: Small
- **Risk**: Low

### Option B: Add jq-with-fallback to all handlers
Apply the `if command -v jq` pattern to all tmux handlers that currently use grep-only.

- **Pros**: Maximum backward compatibility
- **Cons**: Preserves broken fallback paths, more code
- **Effort**: Medium
- **Risk**: None

## Technical Details

- **Affected files**:
  - `crates/mika-agent/templates/skills/tmux/handlers/send_command.sh`
  - `crates/mika-agent/templates/skills/tmux/handlers/kill_session.sh`
  - `crates/mika-agent/templates/skills/tmux/handlers/read_output.sh`
  - `crates/mika-agent/templates/skills/tmux/handlers/wait_for_text.sh`
  - `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` (remove grep fallback if Option A)

## Acceptance Criteria

- [ ] All handler scripts use the same JSON parsing approach
- [ ] No handler silently truncates fields containing double quotes
- [ ] Startup warning if jq is not found on PATH
- [ ] All existing tests pass
