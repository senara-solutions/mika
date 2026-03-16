---
status: pending
priority: p2
issue_id: "683"
tags: [code-review, ux, cli]
dependencies: []
---

# --no-interactive flag on teams create is a dead flag

## Problem Statement

`mika teams create <name> --no-interactive` is accepted by clap but always triggers an immediate bail: "Team creation requires an interactive terminal." The flag's only effect is to force an error. This is misleading — users expect a flag to enable a behavior, not guarantee failure.

Additionally, the old team create path (raw stdin) was technically usable with piped input, which is now a behavioral regression.

## Findings

- **Pattern Recognition**: `--no-interactive` is the only negative flag in the CLI. It creates double-negation at usage site (`!no_interactive`). Consider `--skip-wizard` or removing the flag from teams.
- **Architecture Strategist**: Asymmetry with agents (which degrade gracefully to defaults in non-interactive mode) vs teams (which bail).

**Affected files:**
- `crates/mika-cli/src/cli.rs` (`TeamsCommand::Create` variant)
- `crates/mika-cli/src/commands/teams.rs` (`create` function)

## Proposed Solutions

### Option A: Remove --no-interactive from TeamsCommand::Create (Recommended)
Teams genuinely require member selection which has no sensible default. Remove the flag and let the TTY check handle it implicitly.
- **Pros:** No misleading flag, cleaner API
- **Cons:** Asymmetric with agents create
- **Effort:** Small
- **Risk:** Low

### Option B: Add --orchestrator and --members flags for non-interactive team creation
Support `mika teams create myteam --no-interactive --orchestrator mika --members worker1,worker2`.
- **Pros:** Full non-interactive support, useful for CI
- **Cons:** More flags to implement and test, roles/mandates still need defaults
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] `mika teams create <name>` in non-TTY context produces a clear error (current behavior) or succeeds with flags (Option B)
- [ ] No misleading flag that always errors
