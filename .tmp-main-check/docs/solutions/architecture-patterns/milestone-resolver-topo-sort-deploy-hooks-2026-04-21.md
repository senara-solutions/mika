---
module: self-dev, mika-agent (tools)
date: 2026-04-21
problem_type: best_practice
component: tooling
severity: medium
tags:
  - milestone
  - topological-sort
  - deploy-hooks
  - sprint-engine
  - dependency-resolution
  - blocked-by
  - graphql
applies_when:
  - Adding dependency-aware ordering to prompt-level orchestration
  - Inserting inter-ticket hooks (build/deploy) in serial execution loops
  - Extracting engine-level functions into shared modules for tool reuse
  - Designing callback routing for multiple long-running handler types
---

# Milestone Resolver: Topo-Sort + Deploy Hooks in Sprint Engine

## Context

The self-dev milestone workflow (M1-M5 in `skills/bundled/self-dev/system_prompt.md`) executed child issues in flat issue-number order, ignoring dependency relationships. The engine-level blocked-by guard (#713) caught violations at dispatch time, but by then the milestone was stalled requiring manual intervention. Additionally, no mechanism existed for inter-ticket build/deploy hooks — deploys only happened at M5 (milestone completion), even when intermediate tickets added runtime behavior that subsequent tickets depended on.

## Guidance

### 1. Use a dedicated engine tool for dependency queries, not `gh api`

The prompt layer cannot call `gh api` (blocked subcommand for security). Rather than opening `gh api` to all queries, create a purpose-built tool that exposes exactly the query needed. `resolve_issue_order` takes a repo and issue list, batch-queries `blockedByIssues` via GraphQL, builds a DAG, and runs Kahn's algorithm — all in deterministic Rust code, not LLM-interpreted.

This follows the "dumb store + smart prompt" boundary: the tool is a pure query (no orchestration), and the prompt decides what to do with the results.

### 2. Extract shared modules when tools and guards need the same infrastructure

`fetch_open_blockers` and `extract_open_blocker_numbers` lived in `skills/executor.rs` (private). The new `resolve_issue_order` tool needed the same GraphQL query. Rather than duplicating or making tools call into the skills module (cross-module dependency), extract into a shared `crate::github_graphql` module imported by both.

### 3. Dispatch long-running deploy hooks on the milestone parent task

`deploy_mika` is a `long_running` handler requiring a task in `pending`/`in_progress` state. After a child's PR merges, dispatch `deploy_mika` with `task_id=milestone_parent` (which stays `in_progress` throughout M4). The callback task becomes a child of the milestone parent, so existing callback routing (`parent_task_id` -> parent `type` = "milestone") detects milestone context automatically.

Key safety properties:
- The claude-pilot callback task is already `completed` (marked by `handle_task_complete` before the agent turn) — global dispatch guard passes
- The milestone URL causes `parse_github_ref` to return `None` — blocked-by guard is correctly skipped
- Per-turn dispatch limit is respected because each callback triggers a new agent turn

### 4. Route callback types by task label, not result text

The executor creates callback tasks with label `long_running:<tool_name>`. Use `check_task` on the callback task itself to read the label for structural routing:
- `long_running:run_claude_pilot` -> claude-pilot handling (metadata extraction)
- `long_running:deploy_mika` -> deploy hook handling (skip metadata, advance M4)

This follows the institutional learning from the callback misrouting incident: "every entry point needs explicit routing."

### 5. Add `task_id` to `tools.json` when dispatch-readiness validation is needed

`deploy_mika`'s schema only declared `cwd`. The LLM won't pass undeclared parameters. Adding `task_id` as optional in the input schema makes it visible to the LLM, matching the `run_claude_pilot` pattern. Without this, the dispatch-readiness guard fails with an empty task_id.

### 6. Use structural GATE checks for every new mandatory step

M2b (dependency resolution) is mandatory — skipping it causes the engine-level guard to catch violations at dispatch time (stalling the milestone). The GATE pattern with verification conditions and incident references is harder for LLMs to skip than prose instructions. This matches the fix for the M2-skip incident.

## Why This Matters

- **Topo-sort prevents dispatch stalls:** Without dependency-aware ordering, the blocked-by guard (#713) fires at dispatch time, stalling the milestone. With topo-sort, issues execute in dependency order and the guard serves as defense-in-depth.
- **Deploy hooks enable runtime-dependent tickets:** Tickets adding new tools, startup code, or runtime behavior can be followed by a deploy before the next ticket that depends on that behavior.
- **Shared modules reduce duplication drift:** `github_graphql` is imported by both the dispatch guard and the resolution tool. A bug fix in the GraphQL query applies to both callers.
- **Label-based routing prevents silent misrouting:** Heuristic text matching on callback bodies is fragile. Task labels are structural and engine-generated.

## When to Apply

- Adding a new long-running handler that must participate in milestone/project loops
- Creating a new builtin tool that needs GitHub API access the prompt layer can't call directly
- Extending callback routing to distinguish between multiple handler types
- Adding inter-ticket orchestration steps to the serial execution loop

## Examples

**Before:** M2 fetches issues sorted by number. M4 dispatches in that order. Issue #5 (blocked by #3) dispatches before #3 → engine guard fires → milestone stalled.

**After:** M2 fetches issues + labels. M2b calls `resolve_issue_order` → returns `[3, 5, ...]`. M3 creates children in topo-sorted order. M4 dispatches #3 first, #5 second. If #3 has `needs-deploy` label, deploy runs between them.
