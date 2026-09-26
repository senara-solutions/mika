---
title: "feat: add git-ops builtin skill for git maintenance operations"
type: feat
status: active
date: 2026-03-27
issue: "#300"
---

# feat: add git-ops builtin skill for git maintenance operations

## Overview

Add a `git-ops` bundled skill with a `git_ops` builtin handler that provides structured, auditable git maintenance operations (rebase, merge, fetch). This replaces ad-hoc `run_shell` git commands with deterministic, env-scrubbed subprocess execution and structured error reporting.

## Problem Statement / Motivation

The self-dev skill launches claude-pilot with a hardcoded `/mika` prompt — correct for feature work (full CE pipeline). But when mika-dev needs pure git maintenance (rebase onto main, sync upstream), the only escape hatch is `run_shell`, which breaks determinism (arbitrary shell, no structured errors, no audit trail).

`git-ops` is the structural fix: a dedicated builtin skill for git maintenance. Self-dev stays single-responsibility (CE pipeline launcher). `git-ops` handles git ops synchronously — no claude-pilot, no worktree, no callback ceremony.

## Proposed Solution

### 1. Bundled skill files

Create `crates/mika-agent/templates/skills/git-ops/` with three files:

**`skill.toml`** — `always_on = false`, `timeout_secs = 120`, keywords targeting git maintenance phrases.

**`tools.json`** — Single `git_ops` tool with builtin handler:
- `operation`: enum `"rebase"` | `"merge"` | `"fetch"` (required)
- `repo_path`: string, absolute path to git repo (required)
- `base`: string, remote ref to target (default: `"origin/main"`, optional)
- `push`: boolean, force-push after rebase (default: `false`, optional)

**`system_prompt.md`** — Usage guidance: when to use git_ops vs run_shell, operation semantics, conflict handling expectations.

### 2. Rust builtin handler

New `git_ops` function in `crates/mika-agent/src/skills/builtin_handlers.rs`:

- **`fetch`** — `git fetch origin` (or extract remote from `base` param)
- **`rebase`** — `git fetch` then `git rebase <base>`. On conflict: auto-abort (`git rebase --abort`), return structured error listing conflicting files
- **`merge`** — `git fetch` then `git merge --ff-only <base>`. Reject `push: true` on merge
- **`push`** — After successful rebase only: `git push --force-with-lease`. Reject push to `main`/`master` branches

**Pre-flight checks before rebase/merge:**
- Verify `repo_path` exists and is a directory
- Verify it's a git repo (`git rev-parse --git-dir`)
- Check for dirty working tree (`git status --porcelain`)
- Check for in-progress rebase/merge (`.git/rebase-apply`, `.git/rebase-merge`, `.git/MERGE_HEAD`)

**Subprocess safety:** Use `tokio::process::Command` with `scrub_mika_env_vars()` + `GIT_TERMINAL_PROMPT=0`. Reuse `spawn_and_collect()` for bounded output capture. Do NOT scrub git auth vars (`GH_TOKEN`, `GITHUB_TOKEN`, `SSH_AUTH_SOCK`, `GIT_SSH_COMMAND`).

### 3. Registration

- Add `"git_ops"` to `KNOWN_BUILTINS` array in `builtin_handlers.rs`
- Add match arm in `execute()` dispatch function
- Add `GIT_OPS_SKILL` static in `bundled_skills.rs` using `skill!` macro
- Add to `BUNDLED_SKILLS` array

## Technical Considerations

### Conflict handling strategy

Auto-abort on conflict is the correct choice for an AI agent that cannot resolve merge conflicts. The handler detects conflicts via non-zero exit + `REBASE_HEAD` presence, runs `git rebase --abort`, and returns a structured error with the list of conflicting files (parsed from stderr). The agent can then inform the user and suggest alternatives.

### Path safety

No path restriction beyond basic validation (must exist, must be a directory, must be a git repo). Container isolation is the primary security boundary in production. In bare-metal CLI mode, the agent already has full filesystem access via `run_shell`. Adding path restrictions here would be inconsistent security theater.

### Push safety

- Always use `--force-with-lease` (never `--force`) — prevents overwriting remote changes
- Reject push when current branch is `main` or `master` — hardcoded safety check
- Reject `push: true` on merge operations — ff-only merge doesn't rewrite history, plain push suffices (but the tool doesn't do plain push to keep scope minimal)

### Non-zero exit code handling

Per learnings from `exec-handler-stdout-discarded-on-nonzero-exit.md`: always capture and return stdout regardless of exit code. Use `ToolOutput::success()` with formatted output (matching `spawn_and_collect` pattern) rather than `ToolOutput::error()` so the agent sees the full git output.

### Keyword selection

Per learnings from `adding-prompt-only-bundled-skill.md`: avoid bare "git" keyword (matches "forget", "digital"). Use specific multi-word phrases:
```
["rebase", "merge main", "sync main", "git sync", "sync branch", "fast-forward", "git fetch", "rebase onto", "git rebase", "git merge"]
```

No overlap with `github` skill keywords (which use "merge pr", "pull request" etc.).

## Acceptance Criteria

- [x] `crates/mika-agent/templates/skills/git-ops/skill.toml` — valid manifest, `always_on = false`, `timeout_secs = 120`
- [x] `crates/mika-agent/templates/skills/git-ops/tools.json` — single `git_ops` tool with builtin handler
- [x] `crates/mika-agent/templates/skills/git-ops/system_prompt.md` — clear usage guidance
- [x] `git_ops` handler in `builtin_handlers.rs` — fetch, rebase, merge operations with pre-flight checks
- [x] Auto-abort on rebase conflict with structured error
- [x] `push: true` uses `--force-with-lease`, rejects push to main/master, rejects push on merge
- [x] MIKA_* env var scrubbing + `GIT_TERMINAL_PROMPT=0` on all subprocesses
- [x] `"git_ops"` in `KNOWN_BUILTINS` and `execute()` dispatch
- [x] `GIT_OPS_SKILL` in `bundled_skills.rs` + `BUNDLED_SKILLS` array
- [x] Unit tests: successful fetch, rebase conflict → auto-abort, unknown operation error, push-on-merge rejection, push-to-main rejection
- [x] `cargo clippy` and `cargo test` pass clean
- [x] `docs/skills.md` updated with git-ops skill entry

## Implementation Phases

### Phase 1: Skill template files (quick)

Create the three template files under `crates/mika-agent/templates/skills/git-ops/`.

**Files:**
- `crates/mika-agent/templates/skills/git-ops/skill.toml`
- `crates/mika-agent/templates/skills/git-ops/tools.json`
- `crates/mika-agent/templates/skills/git-ops/system_prompt.md`

### Phase 2: Builtin handler implementation (core work)

Implement `git_ops` in `builtin_handlers.rs`:

1. Add `"git_ops"` to `KNOWN_BUILTINS`
2. Add match arm in `execute()`
3. Implement input validation (parse operation, repo_path, base, push)
4. Implement pre-flight checks (is dir, is git repo, clean tree, no in-progress ops)
5. Implement `fetch` operation
6. Implement `rebase` operation with conflict detection + auto-abort
7. Implement `merge` operation (ff-only)
8. Implement `push` after rebase with safety checks

**Key reference:** Follow `run_gh` pattern for subprocess execution. Use `spawn_and_collect()` for all git commands.

**Files:**
- `crates/mika-agent/src/skills/builtin_handlers.rs`

### Phase 3: Bundled skill registration

Register in `bundled_skills.rs`:
1. Add `static GIT_OPS_SKILL` using `skill!` macro
2. Add `&GIT_OPS_SKILL` to `BUNDLED_SKILLS` array

**Files:**
- `crates/mika-agent/src/bundled_skills.rs`

### Phase 4: Tests

Add unit tests in `builtin_handlers.rs` (inline `#[cfg(test)] mod tests`):
1. Input validation tests (missing operation, unknown operation, missing repo_path)
2. Pre-flight check tests (non-existent path, not a git repo)
3. Push safety tests (push on merge → rejected, push to main → rejected)
4. Integration-style test using a temp git repo for successful fetch/rebase

**Files:**
- `crates/mika-agent/src/skills/builtin_handlers.rs` (test module)

### Phase 5: Documentation

Update `docs/skills.md` with git-ops skill entry following existing format.

**Files:**
- `docs/skills.md`

## Sources & References

### Internal References
- Builtin handler dispatch: `crates/mika-agent/src/skills/builtin_handlers.rs:36-70`
- Bundled skill registration: `crates/mika-agent/src/bundled_skills.rs:28-133`
- Git subprocess pattern: `crates/mika-agent/src/skills/git.rs:164-176`
- Env scrubbing: `crates/mika-agent/src/skills/executor.rs:29-46`
- spawn_and_collect: `crates/mika-agent/src/skills/builtin_handlers.rs:325-396`
- GitHub skill reference: `crates/mika-agent/templates/skills/github/`

### Institutional Learnings
- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — MIKA_* scrubbing tiers
- `docs/solutions/integration-issues/adding-prompt-only-bundled-skill.md` — bundled skill registration pattern
- `docs/solutions/runtime-errors/builtin-handler-timeout-ignores-skill-config.md` — timeout chain
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` — domain-scoped naming
- `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md` — non-zero exit handling

### Related
- GitHub issue: #300
