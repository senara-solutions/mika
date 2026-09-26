---
title: "feat: add pull, checkout, and worktree operations to bundled git-ops skill"
type: feat
status: active
date: 2026-04-21
issue: "#610"
---

# feat: add pull, checkout, and worktree operations to bundled git-ops skill

## Overview

Extend the bundled `git-ops` skill with six new operations: `pull`, `checkout`, `worktree_add`, `worktree_remove`, `worktree_list`, and `worktree_prune`. This closes the gap where agents fall back to `run_shell` for branch switching and worktree management, bypassing git-ops' security boundary (path validation, env scrubbing, audit logging).

## Problem Frame

The git-ops skill currently supports only `fetch`, `rebase`, `merge`. Agents needing to pull, checkout branches, or manage worktrees must use `run_shell` with raw git commands. This bypasses:
- Absolute path validation (security)
- MIKA_* env var scrubbing (secret leak prevention)
- GIT_TERMINAL_PROMPT=0 (credential prompt suppression)
- Structured error reporting (agent-friendly output)
- Audit trails via tool call storage

Issue #610 re-filed from mika-skills#97 after git-ops moved to bundled in mika#601.

## Requirements Trace

- R1. `pull` fetches and fast-forward merges in one call; fails cleanly if not ff-able
- R2. `checkout` switches to a local or remote-tracking branch; fails cleanly if branch doesn't exist
- R3. `worktree_add` creates a worktree with optional new branch
- R4. `worktree_remove` removes a worktree (with --force)
- R5. `worktree_list` lists worktrees in porcelain format
- R6. `worktree_prune` cleans up stale worktree references
- R7. Existing operations (fetch, rebase, merge) unchanged — no regression
- R8. All new operations inherit git-ops security: absolute path validation, env scrubbing, argument injection prevention

## Scope Boundaries

- No `git branch` creation/deletion (agents use `run_shell` or `run_gh` for that)
- No `git stash` operations
- No `git cherry-pick` or `git tag`
- Worktree path validation: absolute paths only, consistent with repo_path
- No changes to skill triggering logic or always_on behavior

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/builtin_handlers.rs` — git_ops handler (line 611), validate_git_ops_input (line 462), git_ops_preflight (line 536), run_git helper (line 432), per-operation functions (git_ops_fetch, git_ops_rebase, git_ops_merge, git_ops_push)
- `crates/mika-agent/templates/skills/git-ops/tools.json` — tool schema with operation enum, repo_path, base, push params
- `crates/mika-agent/templates/skills/git-ops/system_prompt.md` — LLM usage guidance
- `crates/mika-agent/templates/skills/git-ops/skill.toml` — manifest with trigger keywords
- Existing validation pattern: `validate_git_ops_input()` returns `Result<GitOpsInput, ToolOutput>` with structured errors
- Existing preflight pattern: `git_ops_preflight()` checks path exists, is dir, is git repo, clean tree, no in-progress ops
- Protected branches constant: `GIT_OPS_PROTECTED_BRANCHES` at line 420

### Institutional Learnings

- `docs/plans/2026-03-27-002-feat-add-git-ops-builtin-skill-plan.md` — original git-ops plan establishes the pattern: builtin handler, structured GitResult, auto-abort on conflict, pre-flight checks, env scrubbing via `scrub_mika_env_vars()`

## Key Technical Decisions

- **New parameters instead of overloading existing ones:** `checkout` needs a `branch` parameter; worktree ops need `path` and `branch` parameters. These are added as optional fields in tools.json (only validated when relevant operation is selected). This follows the existing pattern where `push` is only valid for `rebase`.
- **`pull` = fetch + merge --ff-only:** Matches the conservative safety posture of the existing `merge` operation. No rebase-pull variant — agents who want fetch+rebase already have the `rebase` operation.
- **`checkout` uses `git switch`:** Prefer `git switch` over `git checkout` — it's the modern, safer command that only switches branches (won't accidentally checkout files). Falls back cleanly on older git versions with clear error.
- **Worktree path must be absolute:** Consistent with `repo_path` validation. Prevents relative path confusion.
- **`worktree_remove` uses --force:** Agents operate in automated contexts where interactive "are you sure?" is not useful. The force flag removes even with uncommitted changes. The agent LLM should decide whether to proceed — the tool trusts its input.
- **No preflight clean-tree check for checkout:** `git switch` already refuses to switch with uncommitted changes that conflict. Let git handle this and return its error message rather than duplicating the check.
- **Preflight for pull:** Same as merge — requires clean working tree and no in-progress rebase/merge.
- **Worktree operations skip clean-tree checks:** Worktree add/remove/list/prune operate on worktree metadata, not the current working tree.

## Open Questions

### Resolved During Planning

- **Should `checkout` create branches?** No. `checkout` switches to existing branches. Branch creation is out of scope (agents use `run_gh` or `run_shell`). If the branch doesn't exist locally but exists on the remote, `git switch` auto-creates a tracking branch.
- **Should `worktree_add` require a base ref?** Made optional with default `HEAD`. The `base` parameter is reused from the existing schema.

### Deferred to Implementation

- **Exact error message formatting for new operations** — will follow the existing pattern (operation-specific prefix + git output) but exact wording determined during implementation.

## Implementation Units

- [ ] **Unit 1: Extend validation and parameter schema**

**Goal:** Add new operations to the validation function and new parameters (`branch`, `path`) to the input struct and tools.json.

**Requirements:** R1, R2, R3, R4, R5, R6, R8

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs` (GitOpsInput struct, validate_git_ops_input fn)
- Modify: `crates/mika-agent/templates/skills/git-ops/tools.json`
- Test: `crates/mika-agent/src/skills/builtin_handlers.rs` (inline tests module)

**Approach:**
- Add `branch: Option<String>` and `path: Option<String>` to `GitOpsInput`
- Extend the operation enum validation to accept: `pull`, `checkout`, `worktree_add`, `worktree_remove`, `worktree_list`, `worktree_prune`
- Validate `branch` is required for `checkout` and `worktree_add`
- Validate `path` is required and absolute for `worktree_add` and `worktree_remove`
- Validate `branch` doesn't start with `-` (argument injection prevention, same as `base`)
- Validate `path` doesn't start with `-`
- `push` remains only valid for `rebase`
- Update tools.json enum and add `branch` and `path` property definitions with descriptions noting which operations use them

**Patterns to follow:**
- Existing `validate_git_ops_input()` pattern for parameter extraction and error returns
- Existing `base` dash-prefix rejection for argument injection prevention

**Test scenarios:**
- Happy path: valid `pull` with repo_path parses correctly with defaults
- Happy path: valid `checkout` with branch parses correctly
- Happy path: valid `worktree_add` with path, branch, and base parses correctly
- Happy path: valid `worktree_list` with only repo_path parses correctly
- Happy path: valid `worktree_prune` with only repo_path parses correctly
- Error path: `checkout` without `branch` returns structured error
- Error path: `worktree_add` without `path` returns structured error
- Error path: `worktree_add` without `branch` returns structured error
- Error path: `worktree_add` with relative `path` returns structured error
- Error path: `worktree_remove` without `path` returns structured error
- Error path: `worktree_remove` with relative `path` returns structured error
- Error path: `branch` starting with `-` is rejected
- Error path: `path` starting with `-` is rejected
- Edge case: `push` on `pull` is rejected
- Edge case: `push` on `checkout` is rejected

**Verification:**
- All new validation tests pass
- Existing validation tests still pass (no regression)

- [ ] **Unit 2: Implement pull and checkout operation handlers**

**Goal:** Add `git_ops_pull` and `git_ops_checkout` handler functions and wire them into the main dispatch.

**Requirements:** R1, R2

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs` (git_ops fn dispatch, new git_ops_pull and git_ops_checkout fns)
- Test: `crates/mika-agent/src/skills/builtin_handlers.rs` (inline tests module)

**Approach:**
- `git_ops_pull(repo_path, remote, base)`: fetch remote, then `git merge --ff-only <base>`. Same logic as merge but combined into a single operation for agent convenience. Preflight: clean tree + no in-progress ops (reuse existing preflight with operation="pull" added to the clean-tree check).
- `git_ops_checkout(repo_path, branch)`: run `git switch <branch>`. On failure, return structured error. No preflight clean-tree check — let git handle conflicts naturally.
- Add `"pull"` and `"checkout"` match arms in `git_ops()` dispatch.
- Add `"pull"` to the preflight clean-tree check alongside `"rebase"` and `"merge"`.

**Patterns to follow:**
- `git_ops_merge()` for pull (fetch + merge --ff-only pattern)
- `git_ops_fetch()` for simple single-command operations (checkout)

**Test scenarios:**
- Happy path: pull on a local repo with no remote fails with structured fetch error (same pattern as existing fetch test)
- Happy path: checkout on a local repo with nonexistent branch returns structured error
- Integration: pull dispatches through git_ops() correctly
- Integration: checkout dispatches through git_ops() correctly

**Verification:**
- `cargo test -p mika-agent` passes with new tests
- Pull combines fetch+merge behavior in a single tool call
- Checkout uses `git switch` and returns git's error messages verbatim

- [ ] **Unit 3: Implement worktree operation handlers**

**Goal:** Add worktree_add, worktree_remove, worktree_list, and worktree_prune handler functions.

**Requirements:** R3, R4, R5, R6

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs` (git_ops fn dispatch, new worktree handler fns)
- Test: `crates/mika-agent/src/skills/builtin_handlers.rs` (inline tests module)

**Approach:**
- `git_ops_worktree_add(repo_path, path, branch, base)`: `git worktree add -b <branch> <path> <base>`. If branch already exists, fall back to `git worktree add <path> <branch>` (same pattern as the /mika worktree setup). Default base: `HEAD`.
- `git_ops_worktree_remove(repo_path, path)`: `git worktree remove --force <path>`.
- `git_ops_worktree_list(repo_path)`: `git worktree list --porcelain`. Porcelain format is machine-readable, better for agent parsing.
- `git_ops_worktree_prune(repo_path)`: `git worktree prune`.
- Add all four match arms in `git_ops()` dispatch.
- Worktree operations do NOT require clean-tree preflight (they operate on worktree metadata).

**Patterns to follow:**
- `git_ops_fetch()` for simple operations returning structured success/error
- `run_git()` for all subprocess execution

**Test scenarios:**
- Happy path: worktree_list on a git repo returns porcelain output
- Happy path: worktree_prune on a clean repo succeeds
- Happy path: worktree_add creates a worktree in a temp directory (end-to-end with real git repo)
- Happy path: worktree_remove removes the worktree created above
- Error path: worktree_add with a path that already exists as a worktree returns structured error
- Error path: worktree_remove with a nonexistent path returns structured error
- Integration: all four worktree operations dispatch through git_ops() correctly

**Verification:**
- `cargo test -p mika-agent` passes with new tests
- Worktree operations work on real temp git repos (not mocked)

- [ ] **Unit 4: Update skill metadata and system prompt**

**Goal:** Update tools.json description, system_prompt.md, and skill.toml keywords to document all new operations.

**Requirements:** R1, R2, R3, R4, R5, R6

**Dependencies:** Units 1-3

**Files:**
- Modify: `crates/mika-agent/templates/skills/git-ops/tools.json` (description text)
- Modify: `crates/mika-agent/templates/skills/git-ops/system_prompt.md`
- Modify: `crates/mika-agent/templates/skills/git-ops/skill.toml` (description, keywords)

**Approach:**
- Update tool description to list all 9 operations
- Add usage examples for pull, checkout, and worktree operations to system_prompt.md
- Add operation documentation sections for each new operation
- Update the "When to Use" and "Important" sections to reflect expanded capability
- Add trigger keywords: `git pull`, `pull main`, `checkout`, `switch branch`, `worktree`, `git worktree`
- Update skill description to mention the full operation set

**Patterns to follow:**
- Existing system_prompt.md format: "When to Use" examples, "Operations" detail sections, "Important" notes

**Test expectation:** none — pure documentation/metadata changes

**Verification:**
- tools.json is valid JSON
- system_prompt.md covers all operations with examples
- skill.toml keywords include trigger phrases for new operations

## System-Wide Impact

- **Interaction graph:** The git_ops builtin handler is invoked by the agent loop's tool dispatch. No callbacks, middleware, or observers are affected. The handler uses `run_git()` which calls `scrub_mika_env_vars()` from the executor module — this dependency is unchanged.
- **Error propagation:** New operations return `ToolOutput::success()` or `ToolOutput::error()` through the existing path. No new error propagation channels.
- **State lifecycle risks:** Worktree operations create/remove filesystem state. `worktree_remove --force` can destroy uncommitted changes — acceptable for agent workflows where the agent is responsible for deciding when to remove.
- **API surface parity:** This is a tool-level change only. No HTTP API, CLI, or dashboard changes needed.
- **Integration coverage:** The end-to-end path (agent loop -> skill dispatch -> builtin handler -> git subprocess) is exercised by the existing git_ops_fetch test pattern. New operations follow the same path.
- **Unchanged invariants:** Existing fetch/rebase/merge operations are not modified. Protected branch checks still apply. Env scrubbing still applies. Path validation still applies.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `git switch` not available on old git versions | `git switch` has been stable since git 2.23 (Aug 2019). All supported platforms have it. If an edge case surfaces, the error message from git is clear. |
| Worktree path conflicts with existing directories | `git worktree add` already handles this — returns a clear error. No extra validation needed. |
| `worktree_remove --force` destroys uncommitted work | Documented in system_prompt.md. Agent LLM is responsible for deciding when to remove. Matches the principle of trusting tool input in automated contexts. |

## Sources & References

- Related issue: #610
- Original git-ops plan: `docs/plans/2026-03-27-002-feat-add-git-ops-builtin-skill-plan.md`
- Predecessor issue: mika-skills#97 (closed)
