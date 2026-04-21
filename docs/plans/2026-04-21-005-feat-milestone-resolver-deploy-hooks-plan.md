---
title: "feat: Milestone resolver + deploy hooks + topo-sort in sprint engine"
type: feat
status: active
date: 2026-04-21
issue: 714
---

# feat: Milestone resolver + deploy hooks + topo-sort in sprint engine

## Overview

Upgrade the milestone workflow in the self-dev skill to resolve ordered ticket lists from GitHub milestones via topological sort on blocked-by relationships, and insert deploy hooks between tickets that carry `needs-build` or `needs-deploy` labels. This replaces hand-assembled ordered lists with "point at a milestone and the agent derives the order."

## Problem Frame

Today, the milestone workflow (M1-M5 in `skills/bundled/self-dev/system_prompt.md`) fetches open issues and sorts them by issue number ascending — a flat ordering that ignores dependency relationships. If issue #5 depends on issue #3, the agent dispatches them in number order by coincidence, but there is no guarantee. The engine-level blocked-by guard (#713) catches violations at dispatch time, but by then the milestone is stalled and requires manual intervention.

Additionally, some tickets require a build or deploy between them — e.g., a ticket adding a new tool that a subsequent ticket's tests depend on at runtime. Currently there is no mechanism for inter-ticket hooks; deploys only happen at M5 (milestone completion).

## Requirements Trace

- R1. Milestone resolution: fetch open issues from a GitHub milestone and order them by dependency (blocked-by edges), not just issue number
- R2. Topological sort: build a DAG from blocked-by relationships, topo-sort with issue-number tiebreaker for ties, detect and report cycles
- R3. Deploy hooks: tickets with `needs-build` or `needs-deploy` labels trigger a build/deploy after their PR merges, before the next ticket dispatches
- R4. End-of-group release: every completed milestone ends with an unconditional deploy (already exists in M5, must be preserved)
- R5. Grouping persistence: store milestone metadata on each work item for resume/re-derive
- R6. Topo-sort validation on manual sprint input: topo-sort always runs, even on inline lists, as a validation pass
- R7. External blockers: issues blocked by tickets outside the milestone are identified and handled gracefully
- R8. The engine remains a "dumb store" — orchestration logic stays in the skill prompt layer

## Scope Boundaries

- Project v2 resolver is **deferred** — the abstraction accommodates it but no implementation until a Projects v2 board exists to test against
- No schema migration — uses existing `tasks.metadata` JSON column
- The existing engine-level blocked-by guard (#713) is unchanged — it serves as defense-in-depth behind the prompt-level ordering

### Deferred to Separate Tasks

- Project v2 resolver: future iteration when a Projects v2 board exists
- Manual sprint topo-sort validation (R6): separate PR — requires a sprint dispatch shape that doesn't exist yet in the current self-dev prompt

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` — Milestone Workflow (M1-M5), Callback Entry Point, Calibration Rules
- `crates/mika-agent/src/skills/executor.rs` lines 558-645 — `fetch_open_blockers()` and `extract_open_blocker_numbers()` — existing GraphQL infrastructure for blocked-by queries
- `crates/mika-agent/src/skills/builtin_handlers.rs` line 978 — `GH_ALLOWED_SUBCOMMANDS` (does NOT include `api`)
- `crates/mika-agent/src/tools/create_task.rs` — `create_task` with `type`, `parent_task_id`, `metadata` support
- `crates/mika-agent/src/task_metadata.rs` — two-level shallow merge for metadata writes
- `skills/bundled/deploy-mika/` — long-running deploy handler with callback
- `skills/bundled/build-mika/` — build skill (timeout 300s)
- `.github/labels.yml` — canonical label taxonomy (missing `needs-build` and `needs-deploy`)

### Institutional Learnings

- **Milestone callback misrouted to Generic Workflow** — every entry point needs explicit routing. Callback Entry Point must check `parent_task_id` -> parent `type`. JSON code blocks mandatory.
- **Milestone skips M2, creates incomplete children** — structural GATE checks with verification conditions are mandatory for new steps. Prose instructions alone get skipped.
- **Tasks.type column orthogonal role** — mika core is a "dumb store". All orchestration in self-dev. `validate_dispatch_readiness` is a function of status alone.
- **Engine guards vs prompt rules** — with-gradient behaviors hold at prompt layer; against-gradient need engine guards. Topo-sort ordering is with-gradient (agent follows instructions) but the dependency data must come from a tool.
- **Blocked-by dispatch guard (#713)** — GraphQL variables for injection safety, fail-open/fail-closed matrix, expensive checks last. `fetch_open_blockers` already exists and can be reused.
- **Phantom retry guard** — metadata writes with "retry" keys rejected during active dispatch. Deploy hook metadata must avoid "retry" in key names.

## Key Technical Decisions

- **New `resolve_issue_order` builtin tool (Rust):** The prompt cannot query GitHub GraphQL directly (`gh api` is a blocked subcommand for security). Rather than opening `gh api` to all queries, create a purpose-built tool that takes a repo and list of issue numbers, batch-queries `blockedByIssues` via GraphQL (reusing `fetch_open_blockers` infrastructure), builds a DAG, runs Kahn's algorithm for topo-sort with issue-number tiebreaker, and returns the sorted order plus any cycle information. This keeps dependency resolution deterministic (not LLM-interpreted) while respecting the "dumb store" boundary — the tool is a pure query, not orchestration. The shared `fetch_open_blockers` and `extract_open_blocker_numbers` functions should be extracted from `executor.rs` into a shared `crate::github_graphql` module (avoids tools-calling-skills cross-module dependency).

- **Deploy hooks via milestone parent task dispatch:** `deploy_mika` is a long-running handler requiring a task in `pending` or `in_progress` state. After a `needs-deploy` child's PR merges, dispatch `deploy_mika` using the milestone parent's task_id (which remains `in_progress` throughout M4). The deploy callback's task becomes a tracked child of the milestone parent (alongside the issue children), so the existing Callback Entry Point routing (check `parent_task_id` -> parent `type` = "milestone") correctly detects milestone context and routes back to M4. The dispatch is safe: by the time the agent's callback turn runs for the completed claude-pilot session, the server handler has already marked the claude-pilot callback task as `completed` (done in `handle_task_complete` before dispatching the agent), so the global single-dispatch guard passes cleanly. The milestone parent's `reference_url` (a milestone URL, not an issue URL) causes `parse_github_ref` to return `None`, so the blocked-by guard is correctly skipped.

- **`deploy_mika` schema update:** `deploy_mika`'s `tools.json` currently only declares `cwd`. For the LLM to pass `task_id`, it must be added to the input schema as an optional parameter. Without it, the LLM will omit `task_id` and the dispatch-readiness guard will fail. This matches the pattern used by `run_claude_pilot` (which declares `task_id` as required in its schema).

- **Deploy callback detection via task label:** The executor creates long-running callback tasks with label `long_running:<tool_name>`. Deploy callbacks have label `long_running:deploy_mika` (vs `long_running:run_claude_pilot` for claude-pilot). The Callback Entry Point should use `check_task` on the callback task to read its label for structural routing — not heuristic text matching on the callback body. This follows the institutional learning that "every entry point needs explicit routing."

- **Labels fetched at M2 time (batch), stored in child metadata:** Fetch `number,title,labels` during M2 and store label names in each child task's metadata as `grouping.labels`. Accept that mid-execution label changes are not picked up — this is a pragmatic tradeoff for simplicity.

- **External blockers placed after all intra-milestone issues:** Issues with open blockers outside the milestone are identified during resolution and placed at the end of the sorted list. The engine-level guard (#713) provides defense-in-depth at dispatch time.

- **Cycle detection errors the milestone:** If blocked-by relationships form a cycle, `resolve_issue_order` returns the cycle details. The prompt notifies Vincent and pauses the milestone. No automatic cycle-breaking.

## Open Questions

### Resolved During Planning

- **How to query blocked-by from prompt level?** New `resolve_issue_order` builtin tool. Avoids opening `gh api` which would bypass the subcommand allowlist security boundary.
- **How to handle deploy callbacks in milestone context?** Dispatch on milestone parent task_id. Callback routing detects milestone context via existing `parent_task_id` check.
- **Labels taxonomy:** `needs-build` and `needs-deploy` must be added to `.github/labels.yml`.
- **Cycle handling:** Error out, report to Vincent, pause milestone. No automatic breaking.
- **End-of-milestone deploy when zero children completed:** Gate on at least one completed child (avoid deploying stale artifacts).

### Deferred to Implementation

- Exact metadata field layout may adjust based on merge behavior with existing claude_pilot metadata
- Whether `build_mika` should be called separately from `deploy_mika` for `needs-build`-only labels, or if `deploy_mika` handler already includes a build step (it does — the handler builds before deploying)

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
M2 (existing)                    NEW tool
  |                                |
  v                                v
fetch issues + labels          resolve_issue_order(repo, issues)
via run_gh                       |
  |                              +-> batch GraphQL: blockedByIssues per issue
  |                              +-> build DAG
  |                              +-> Kahn's topo-sort (tie: issue# asc)
  |                              +-> detect cycles, identify external blockers
  |                              |
  v                              v
milestone_issues        { sorted: [...], external_blockers: {...}, cycles: null }
(with labels)                    |
  |                              |
  +--------- MERGE -------------+
  |
  v
ordered_issues (topo-sorted, with labels and external-blocker annotations)
  |
  v
M3: create children in topo-sorted order, store grouping metadata
  |
  v
M4: serial loop
  |
  +---> child PR merges
  |       |
  |       +--[has needs-build/needs-deploy label]--> deploy_mika(task_id=milestone_wi) --> wait callback --> next child
  |       |
  |       +--[no label]--> next child directly
  |
  v
M5: end-of-milestone unconditional deploy (gated on >=1 completed child)
```

## Implementation Units

- [x] **Unit 1: Add `needs-build` and `needs-deploy` labels to taxonomy**

  **Goal:** Create the labels that deploy hooks depend on.

  **Requirements:** R3

  **Dependencies:** None

  **Files:**
  - Modify: `.github/labels.yml`

  **Approach:**
  - Add `needs-build` and `needs-deploy` labels to the `# -- Sprint Hooks --` section
  - Color: use a distinct color from existing categories (e.g., orange `"d97706"`)
  - Description: clearly state the inter-ticket hook behavior

  **Patterns to follow:**
  - Existing label format in `.github/labels.yml`

  **Test expectation:** none -- label taxonomy file, no behavioral change

  **Verification:**
  - Labels file is valid YAML with the two new entries

- [x] **Unit 2: Create `resolve_issue_order` builtin tool**

  **Goal:** Provide the prompt layer with a deterministic way to get dependency-aware issue ordering without opening `gh api`.

  **Requirements:** R1, R2, R7

  **Dependencies:** None (parallel with Unit 1)

  **Files:**
  - Create: `crates/mika-agent/src/tools/resolve_issue_order.rs`
  - Create: `crates/mika-agent/src/github_graphql.rs` (shared module — `fetch_open_blockers` and `extract_open_blocker_numbers` extracted from executor.rs)
  - Modify: `crates/mika-agent/src/tools/mod.rs` (register module)
  - Modify: `crates/mika-agent/src/tools.rs` or equivalent tool registration site (add to `default_tools()`)
  - Modify: `crates/mika-agent/src/skills/executor.rs` (replace inline functions with imports from `crate::github_graphql`)
  - Modify: `crates/mika-agent/src/lib.rs` (add `mod github_graphql`)
  - Test: `crates/mika-agent/src/tools/resolve_issue_order.rs` (inline `#[cfg(test)]` module)

  **Approach:**
  - Input schema: `{ repo: "owner/repo", issues: [1, 2, 3] }` — repo as `owner/repo` string, issues as array of issue numbers
  - For each issue, call `fetch_open_blockers` (or extracted shared helper) to get blocked-by edges. Use `ctx.github_token` for auth. Batch sequentially (GitHub rate limit is generous for 10-20 issues)
  - Build adjacency list from edges. Filter to only intra-list edges (blockers outside the issue list are "external")
  - Run Kahn's algorithm: initialize in-degree map, seed queue with zero-in-degree nodes, pop from queue using `BinaryHeap` with issue-number-ascending ordering for deterministic tiebreaking
  - If the sorted output has fewer items than input, a cycle exists — collect remaining nodes as cycle members
  - Return JSON: `{ sorted: [numbers], edges: { "5": [3, 4] }, external_blockers: { "7": [999] }, cycle: [numbers] | null }`
  - Follow existing tool patterns: `#[async_trait]` impl, `ToolContext` with `github_token`, structured JSON errors
  - Fail-open when no GitHub token (return issues in input order with a warning)
  - Timeout: 60s (up to 20 sequential GraphQL calls at ~2s each)

  **Patterns to follow:**
  - `crates/mika-agent/src/tools/check_task.rs` for tool structure
  - `crates/mika-agent/src/skills/executor.rs` `fetch_open_blockers()` for GraphQL calling pattern
  - `extract_open_blocker_numbers()` for response parsing

  **Test scenarios:**
  - Happy path: 5 issues with linear dependency chain (A->B->C->D->E) -> sorted in dependency order
  - Happy path: 5 issues with no dependencies -> sorted by issue number ascending
  - Happy path: diamond dependency (A->B, A->C, B->D, C->D) -> D first, then B/C by number, then A
  - Edge case: single issue -> returns that issue
  - Edge case: empty issue list -> returns empty sorted list
  - Edge case: issue blocked by issue not in the list (external blocker) -> external_blockers populated, sorted list excludes that edge
  - Error path: cycle detected (A->B, B->A) -> cycle field populated with the cycle members, sorted contains only non-cycle nodes
  - Error path: no GitHub token -> returns issues in input order with warning message
  - Integration: verify `fetch_open_blockers` is called with correct parameters for each issue

  **Verification:**
  - `cargo test -p mika-agent -- resolve_issue_order` passes
  - Tool registered and appears in agent tool list

- [x] **Unit 3: Update Milestone Workflow (M2) for label fetching and dependency resolution**

  **Goal:** Modify M2 to fetch labels alongside issues and call `resolve_issue_order` for dependency-aware ordering.

  **Requirements:** R1, R2, R3, R7

  **Dependencies:** Unit 2 (tool must exist)

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (M2 section)

  **Approach:**
  - Update the `run_gh` command in M2 to fetch `number,title,labels` (add `labels` to `--json`)
  - After fetching issues, extract label names per issue and store as a lookup table
  - Add a new **Step M2b — Resolve dependency order (MANDATORY)** after M2:
    - Call `resolve_issue_order` with the repo and issue numbers from M2
    - If `cycle` is non-null: notify Vincent with cycle details, pause milestone, STOP
    - If `external_blockers` is non-empty: log a warning noting which issues have external blockers (they will be placed at the end)
    - Replace `milestone_issues` with the `sorted` order from the tool response
  - Add GATE check: "If you did not call `resolve_issue_order`, you MUST call it now before proceeding to M3"
  - Include incident reference to anchor the instruction

  **Patterns to follow:**
  - Existing M2 GATE pattern ("If milestone_issues is empty, notify Vincent and stop")
  - JSON code blocks for all tool calls (Calibration Rule 4)

  **Test scenarios:**
  - Happy path: milestone with 5 issues, some with blocked-by -> issues created in topo-sorted order in M3
  - Happy path: milestone with 3 issues, no dependencies -> order is issue-number ascending (same as today)
  - Edge case: `resolve_issue_order` returns cycle -> milestone paused with notification
  - Edge case: `resolve_issue_order` returns external blockers -> warning logged, execution continues
  - Error path: `resolve_issue_order` fails (no GitHub token) -> fallback to M2's issue-number order with warning

  **Verification:**
  - Prompt instructions include the new M2b step with GATE check
  - JSON examples show correct `resolve_issue_order` call format

- [x] **Unit 4: Update Milestone Workflow (M3) for grouping metadata**

  **Goal:** Persist milestone grouping metadata on each child task for resume/re-derive.

  **Requirements:** R5

  **Dependencies:** Unit 3 (M2 changes feed into M3)

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (M3 section)

  **Approach:**
  - After each `create_task` call in M3, immediately call `update_task_status` with metadata:
    ```json
    {
      "grouping": {
        "kind": "milestone",
        "repo": "senara-solutions/<repo>",
        "number": <n>,
        "title": "<milestone title>"
      },
      "labels": ["needs-deploy", "enhancement"]
    }
    ```
  - The `grouping` key uses the top-level metadata namespace — no collision with `claude_pilot` (nested) or retry counters (top-level but different keys)
  - `labels` stored at top-level for easy access during M4 deploy-hook checks
  - Milestone title: fetch via `run_gh("milestone list --json number,title --jq '.[] | select(.number==<n>) | .title'")` in M2 (one additional query)

  **Patterns to follow:**
  - Existing metadata write pattern in "Metadata extraction" section of self-dev prompt
  - Two-level shallow merge semantics (see `task_metadata.rs`)

  **Test scenarios:**
  - Happy path: each child task has `grouping` and `labels` metadata after M3
  - Edge case: child task already exists (pre-flight dedup) -> metadata still written via `update_task_status`
  - Integration: metadata merges correctly with subsequent `claude_pilot` metadata writes (no field collision)

  **Verification:**
  - Prompt includes JSON examples for the metadata write
  - GATE verifies metadata was written (check via `check_task`)

- [x] **Unit 5: Update Milestone Workflow (M4) for deploy hooks**

  **Goal:** Insert build/deploy hooks between tickets based on labels.

  **Requirements:** R3

  **Dependencies:** Units 3, 4 (labels and grouping metadata must be in place)

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (M4 section and Callback Entry Point)
  - Modify: `skills/bundled/deploy-mika/tools.json` (add `task_id` to input schema)

  **Approach:**
  - **Schema update:** Add `task_id` (optional string) to `deploy_mika`'s `tools.json` input schema. Without this, the LLM will not pass it and the dispatch-readiness guard will fail. Follows the `run_claude_pilot` pattern.
  - After a child task reaches `completed` (PR merged) in M4 step 3, add a new check:
    1. Read the child's metadata `labels` field
    2. If labels include `needs-build` or `needs-deploy`, invoke deploy hook before advancing to next child
  - **Deploy hook sequence:**
    1. Notify Vincent: "Deploy hook triggered for <repo> issue#<N> (label: needs-deploy). Running build+deploy before next ticket."
    2. Call `deploy_mika` with `task_id` set to the **milestone parent** task_id (milestone parent is `in_progress`, satisfies dispatch guard). The deploy callback task becomes a tracked child of the milestone parent — distinct from the issue children.
    3. Wait for deploy callback (this arrives as a new agent turn)
    4. On deploy callback success: log fact, advance to next child in M4
    5. On deploy callback failure: notify Vincent, pause milestone
  - **Callback Entry Point changes:**
    - Generalize the opening line from "callback result from a completed `run_claude_pilot`" to "callback result from a completed background task"
    - Add **label-based** callback type detection: call `check_task` on the callback task itself and read its `label` field. The executor creates callbacks with label `long_running:<tool_name>`. Route by label:
      - `long_running:run_claude_pilot` → existing claude-pilot callback handling (metadata extraction, etc.)
      - `long_running:deploy_mika` → deploy hook callback: skip metadata extraction (no session data), check milestone context via `parent_task_id` as normal, on success advance to next child in M4
    - This is structural routing (per institutional learning), not heuristic text matching
  - **Interaction with dispatch guard:** The global single-dispatch guard prevents deploy_mika from running while a claude-pilot session is active (and vice versa). By the time the agent's callback turn runs for the completed claude-pilot, the server handler has already marked that callback task `completed` — the guard passes cleanly. Each deploy hook callback triggers a new agent turn; the M4 loop does not continue within a single turn.

  **Patterns to follow:**
  - Existing M4 child outcome table (completed/blocked/failed actions)
  - Existing M5 deploy invocation pattern
  - JSON code blocks for all tool calls

  **Test scenarios:**
  - Happy path: child with `needs-deploy` label completes -> deploy_mika called -> callback received -> next child dispatched
  - Happy path: child without deploy labels completes -> next child dispatched immediately (no deploy hook)
  - Happy path: child with `needs-build` label -> same deploy hook behavior (deploy_mika handler includes build)
  - Edge case: deploy callback fails -> milestone paused, Vincent notified
  - Edge case: both `needs-build` and `needs-deploy` on same issue -> single deploy hook (not two)
  - Integration: deploy_mika dispatched on milestone parent task_id -> dispatch guard accepts (parent is in_progress)
  - Integration: deploy callback routing detects milestone context via parent_task_id check

  **Verification:**
  - Prompt includes clear conditional logic with JSON examples
  - Deploy hook sequence has its own GATE ("If deploy was required and you did not call deploy_mika, STOP and call it now")

- [x] **Unit 6: Update M5 end-of-milestone deploy gate**

  **Goal:** Gate the end-of-milestone deploy on at least one child completing successfully.

  **Requirements:** R4

  **Dependencies:** Unit 5

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (M5 section)

  **Approach:**
  - Before the existing build+deploy step in M5, add: "If zero children completed successfully (all failed/blocked/cancelled), skip the build+deploy and note in the summary: 'No deploy — no children completed.'"
  - This prevents deploying stale artifacts when a milestone was abandoned

  **Patterns to follow:**
  - Existing M5 stats gathering step

  **Test scenarios:**
  - Happy path: 3/5 children completed -> deploy runs as before
  - Edge case: 0/5 children completed (all failed) -> deploy skipped, summary notes it
  - Edge case: 1/5 completed, rest blocked -> deploy runs (at least one success)

  **Verification:**
  - Prompt includes the conditional gate before the deploy step

- [x] **Unit 7: Update Resume Semantics for grouping metadata**

  **Goal:** Enable resumed milestones to re-derive remaining work from grouping metadata.

  **Requirements:** R5

  **Dependencies:** Unit 4 (grouping metadata must be stored)

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (Resume Semantics section)

  **Approach:**
  - In the "Milestone/Project Resume" section, add: "When resuming a milestone, read the parent task's children via `list_tasks(parent_task_id=<milestone_wi>)`. Each child's `metadata.grouping` contains the milestone context. The children are already in the order they were created (topo-sorted at M3 time). Find the next child by priority: `in_progress` > `blocked` > `pending`. Resume from M4."
  - The topo-sort order is already baked into the child creation order — no need to re-sort on resume
  - If new issues were added to the milestone since the sprint started, the resumed sprint does not pick them up (consistent with existing behavior — the sprint snapshot is taken at M2 time)

  **Patterns to follow:**
  - Existing Resume Semantics section structure

  **Test scenarios:**
  - Happy path: milestone paused at child 3/5 -> resume finds child 3 as in_progress, resumes M4
  - Edge case: milestone paused due to deploy hook failure -> resume finds no pending deploy, resumes from next child
  - Edge case: all children completed but M5 not reached -> resume runs M5

  **Verification:**
  - Resume instructions reference grouping metadata and child ordering

## System-Wide Impact

- **Interaction graph:** `resolve_issue_order` (new tool) -> GitHub GraphQL API -> self-dev prompt (M2b) -> `create_task` (M3, ordered) -> `run_claude_pilot` / `deploy_mika` (M4). The `deploy_mika` callback re-enters the Callback Entry Point which routes to M4.
- **Error propagation:** `resolve_issue_order` errors (no token, API failure) fall back to issue-number ordering with a warning — graceful degradation. Cycle detection pauses the milestone. Deploy hook failures pause the milestone.
- **State lifecycle risks:** The global single-dispatch guard prevents concurrent `deploy_mika` and `run_claude_pilot`. Deploy hooks add a second callback phase per child for labeled issues — the M4 loop must handle this two-callback sequence without stalling.
- **API surface parity:** The Project Workflow (P1-P5) has the same shape as Milestone but uses GraphQL for issue fetching. The deploy hook changes in M4 should be mirrored in P4. The `resolve_issue_order` tool works for both (takes repo + issue numbers, agnostic to source). However, updating P4 is out of scope for this PR — it's a separate task.
- **Unchanged invariants:** The engine-level blocked-by guard (#713) is untouched. The dispatch-readiness guard chain is unchanged. The `create_task` dedup and depth guards are unchanged. The metadata merge semantics are unchanged. The `GH_ALLOWED_SUBCOMMANDS` list is unchanged (no new subcommands added).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| GitHub GraphQL rate limiting on batch blocked-by queries (10-20 calls per milestone) | Sequential calls with 10s timeout each. GitHub's rate limit is 5000 points/hour; each query costs ~1 point. A 20-issue milestone uses <1% of the budget. |
| LLM skips the new M2b step (same pattern as the M2-skip incident) | Structural GATE with verification condition and incident reference. Same defense pattern that fixed the original M2-skip. |
| Deploy hook callback misrouted (not recognized as milestone context) | Dispatch deploy_mika on milestone parent task_id, so the callback task's parent IS the milestone. Existing routing logic handles it. |
| `deploy_mika` dispatch rejected because milestone parent already has an active callback child (the just-completed claude-pilot callback) | Safe by construction: `handle_task_complete` marks the claude-pilot callback `completed` before dispatching the agent turn. By the time the agent reaches the deploy hook dispatch, the guard passes. Claude-pilot callbacks are children of the child issue task (not the milestone parent), so guard #2 (active callback child of dispatch task) is also clear. |
| Stale label data (label added mid-milestone not picked up) | Accepted tradeoff. Labels are fetched once at M2 time. Document this limitation in the prompt. |

## Depends On

- mika issue#713 (pre-dispatch blocked-by guard) — already merged, provides the `fetch_open_blockers` infrastructure reused by Unit 2

## Sources & References

- Related issue: [#714](https://github.com/senara-solutions/mika/issues/714)
- Related issue: [#713](https://github.com/senara-solutions/mika/issues/713) (blocked-by guard, provides GraphQL infrastructure)
- Related solution: `docs/solutions/602-milestone-project-workflow-implementation.md`
- Related solution: `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md`
- Related solution: `docs/solutions/logic-errors/milestone-skips-m2-creates-incomplete-children.md`
- Related solution: `docs/solutions/architecture-patterns/blocked-by-dispatch-guard-graphql-validation-2026-04-21.md`
- Existing code: `crates/mika-agent/src/skills/executor.rs` lines 558-645 (`fetch_open_blockers`)
- Existing code: `skills/bundled/self-dev/system_prompt.md` (Milestone Workflow M1-M5)
- Existing code: `skills/bundled/deploy-mika/` (deploy handler)
