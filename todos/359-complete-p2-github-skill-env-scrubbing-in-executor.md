---
status: complete
priority: p2
issue_id: 359
tags: [code-review, security, github-skill]
dependencies: []
---

# Scrub MIKA_* Env Vars in All Exec Handlers (Executor Level)

## Problem Statement

The exec handler in `crates/mika-agent/src/skills/executor.rs` only strips `TMUX` and `TMUX_PANE` from child process environments. Sensitive variables (`MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`, `MIKA_OPENAI_API_KEY`, `MIKA_BRAVE_API_KEY`) are inherited by all exec handler subprocesses.

The GitHub skill handler addresses this locally via `unset` in the shell script, but other exec handlers (tmux, shell-exec, file-reader) remain exposed.

## Findings

- Discovered during PR #36 code review (security-sentinel agent)
- The GitHub skill handler scrubs env vars locally (`unset MIKA_*`)
- Broader fix should be at the executor level for all exec handlers
- This is a systemic improvement, not blocking for the GitHub skill PR

## Recommended Action

Add `env_remove` calls for sensitive `MIKA_*` variables in `executor.rs` `execute_exec()` function for all exec handlers.

## Technical Details

- **Affected file:** `crates/mika-agent/src/skills/executor.rs:240-248`
- **Scope:** All exec handler skills (tmux, shell-exec, file-reader, github)

## Work Log

- 2026-02-28: Created during GitHub skill PR #36 review. Handler-level fix applied. Executor-level fix deferred to follow-up.
