---
title: "run_gh allowlist contained hallucinated subcommands and was missing gh api"
date: 2026-05-17
module: "crates/mika-agent/skills/builtin_handlers"
problem_type: logic_error
component: tooling
severity: medium
symptoms:
  - "Agent calls run_gh(['milestone', ...]) which passes allowlist validation but fails at gh CLI with 'unknown command'"
  - "Agent has no structural path to close a GitHub milestone — milestone #15 sat open despite all children being merged"
  - "gh api not in allowlist prevents REST/GraphQL mutations needed for milestone lifecycle"
root_cause: logic_error
resolution_type: code_fix
tags:
  - run-gh
  - allowlist
  - gh-cli
  - milestone
  - gh-api
  - audit-event
related_components:
  - documentation
---

# run_gh Allowlist: Hallucinated Subcommands and Missing gh api

## Problem

`GH_ALLOWED_SUBCOMMANDS` in `builtin_handlers.rs` listed `milestone` and `project` as allowed `gh` subcommands. Neither exists as a native `gh` CLI command — `gh --help` lists no such subcommands. Any agent call to `run_gh(["milestone", ...])` passed the allowlist check but failed at the `gh` binary with "unknown command."

Meanwhile, `api` was absent from the allowlist, meaning agents had no structural mechanism to call `gh api` for REST or GraphQL operations — the only path to close a GitHub milestone programmatically.

## Root Cause

The allowlist was hand-curated when `run_gh` was first introduced (commit `f3e93e78c`, 2026-03-13). The author conflated GitHub's web-UI concepts (Milestones page, Projects board) with `gh` CLI subcommands. The other eight entries (`pr, issue, run, workflow, release, repo, search, label`) all map to real subcommands and were unaffected.

## What Didn't Work

No prior failed approaches — this was discovered during planning for the self-dev verify-post-state ticket when the milestone #15 repro surfaced the gap. A `git blame` confirmed the entries were original to the handler introduction, not a later regression.

## Solution

Three changes in `builtin_handlers.rs`:

1. **Remove `milestone` and `project`** from `GH_ALLOWED_SUBCOMMANDS`. No deprecation cycle — nothing was using them successfully.

2. **Add `api`** to the allowlist. `gh api` covers both REST mutations (e.g., `gh api --method PATCH /repos/{owner}/{repo}/milestones/{N} -f state=closed`) and GraphQL introspection.

3. **Add `gh_api_invocation` structured audit event** — emitted via `tracing::info!` before every `gh api` subprocess spawn, with `session_id`, `method`, and `path` fields. Provides post-hoc observability for the expanded security surface per the engine-guards-vs-prompt-rules principle (observability binding, not validator gating).

Supporting changes:
- `extract_api_method()` helper: one-pass scan tolerant of `--method X`, `--method=X`, and `-X` shorthand; defaults to `"GET"`.
- Updated 6 skill prompt files that enumerate the permitted subcommand list.
- Updated `docs/skills.md` documentation.
- 4 new tests: rejects removed subcommands, accepts api, extract_api_method forms, accepts realistic PATCH args.

## Why This Works

The two phantom entries never worked — removing them prevents silent failures. Adding `api` provides a constrained path (more constrained than `run_shell`, scoped to GitHub host with `$GH_TOKEN` auth) for the mutation classes the autonomous loop needs (milestone close, project item updates). The audit event makes every `gh api` call greppable for post-hoc detection of misuse patterns.

## Prevention

1. **Cross-check allowlists against actual CLI output.** When hand-curating a command allowlist, verify each entry exists: `gh <subcommand> --help` should not return "unknown command."

2. **Test both positive and negative cases.** The existing `test_run_gh_allowlist_accepts_valid` only tested that allowed subcommands pass. Adding `test_run_gh_allowlist_rejects_removed_subcommands` guards against future re-introduction of phantom entries.

3. **Audit event for security surface expansion.** When adding a broad capability (like `gh api`) to a constrained allowlist, ship observability atomically. The `gh_api_invocation` log event was shipped in the same PR as the allowlist change, not as a follow-up.

## Related

- [GitHub skill missing label documentation](../integration-issues/github-skill-missing-label-documentation.md) — prior precedent on allowlist/docs drift
- [Engine guards vs prompt rules](../architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md) — structural-binding principle applied as observability binding in this fix
- mika#788 — the tracking issue
- mika#1167 — deferred per-method tightening of `gh api` (opens only under concrete triggers)
