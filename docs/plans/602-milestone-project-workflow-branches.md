# Plan: Add milestone and project workflow branches to self-dev

## Issue
https://github.com/senara-solutions/mika/issues/602

## Summary
Teach `self-dev` (now bundled in `mika/skills/bundled/self-dev/`) how to orchestrate milestone and project dispatches using the `tasks.type` column from mika#595.

## Pre-requisites (LIVE)
- ✅ mika#595 - `tasks.type` column
- ✅ mika#598 - `skills/bundled/` build.rs discovery  
- ✅ mika#601 - self-dev migration into bundled

## Design Decisions

### self-dev-sprint fate: DELETE and fold into self-dev

The `self-dev-sprint` skill is redundant. Its "Sprint/Batch Mode" workflow handles numbered lists of issues sequentially. The milestone/project workflows are the natural evolution:

| Old | New |
|-----|-----|
| `self-dev-sprint`: numbered list of issues | `self-dev`: `implement milestone <repo>#<n>` |
| Manual issue list | Auto-fetched from GitHub milestone/project |
| Sprint ID tagging | Parent work item with children |

The sprint skill's serial execution loop, retry handling, and completion summary are all reusable. We'll fold the relevant patterns into self-dev's main workflow and delete `self-dev-sprint/` entirely.

## Implementation

### 1. Update `mika/skills/bundled/self-dev/system_prompt.md`

Add two new workflow branches after the existing per-issue flow:

#### Branch: `implement milestone <repo>#<n>`

```
When the user says "implement milestone <repo>#<n>":

1. Create parent work item: `type='milestone'`, `label='<repo>#<n>'`, `reference_url='https://github.com/senara-solutions/<repo>/milestone/<n>'`
2. Fetch milestone issues: `gh api repos/senara-solutions/<repo>/milestones/<n>/issues?state=open&sort=created&direction=asc`
3. For each issue in order:
   a. Create child work item: `type='issue'`, `parent_task_id=<milestone_wi>`, `label='<repo>#<issue>'`, `reference_url='https://github.com/.../issues/<issue>'`
   b. Execute per-issue flow (Steps 1-6 from existing workflow)
   c. On success: continue to next issue
   d. On failure (QA block, exhausted retries): notify Vincent, pause milestone (parent → `blocked`), stop
4. When all children complete: transition parent to `completed`, notify Vincent

Resume semantics: If re-invoked while milestone parent is `in_progress` or `blocked`:
- Find parent via `list_work_items`, read state
- Find first child (by `parent_task_id`) not `completed` — resume there
- If no next child, close parent as `completed`
```

#### Branch: `implement project <n>`

Same shape, but step 2 uses GitHub Projects v2 GraphQL:

```
gh api graphql -f query='
query {
  organization(login:"senara-solutions") {
    projectV2(number:<n>) {
      items(first:100) {
        nodes {
          content {
            ... on Issue {
              number
              repository { name }
              state
              url
            }
          }
        }
      }
    }
  }
}'
```

Filter to `state=OPEN`, preserve order. Items may span repos.

### 2. Fold sprint patterns into self-dev

From `self-dev-sprint`, migrate these patterns:
- Serial execution with resume after block/failure
- Retry tracking (`pipeline_retry_count`, `qa_retry_count`, `ci_fix_count`)
- Sprint completion summary (adapted for milestone/project)
- Stop/continue commands

### 3. Delete `self-dev-sprint/`

Remove the entire directory: `mika/skills/bundled/self-dev-sprint/`

### 4. Update `self-dev-webhook-qa/system_prompt.md`

Light terminology pass:
- Replace "sprint" references with "milestone/project" where applicable
- Ensure webhook handling for child work items (by `parent_task_id`) works correctly

### 5. Update CLAUDE.md

Document the new `skills/` directory structure from mika#598 and the skills system.

## Test Plan

1. Unit test: Verify `create_work_item` accepts `type='milestone'` and `type='project'`
2. Integration test: Mock `gh api` calls, verify work item tree creation
3. Manual: `implement milestone mika#6` creates expected work-item tree (verified via DB query)

## Acceptance Criteria

- [ ] `implement milestone mika#6` creates parent work item + child issues
- [ ] First child dispatches claude-pilot; QA webhook path fires correctly  
- [ ] Parent work item transitions correctly through lifecycle
- [ ] `grep -i sprint mika/skills/bundled/self-dev*/system_prompt.md` returns only historical references
- [ ] `self-dev-sprint/` directory removed
- [ ] Doc audit pass (CLAUDE.md updated)

## Out of Scope

- Behavioral changes to per-issue flow
- qa-review changes (PR-scoped, unaffected)
- Parallelism — strictly sequential
- Mid-run milestone/project refresh
- Meta-repo `/mika` dispatcher changes
