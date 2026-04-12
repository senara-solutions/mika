# Brainstorm: Task-Based Sprint Mode

**Date:** 2026-04-12
**Status:** Draft
**Author:** Vincent + Claude

## What We're Building

Replace the current prompt-driven sprint mode (mika-dev tracks sprint state in core memory, loops through issues serially) with a task-engine-backed approach. When mika-dev starts a sprint, it creates all sprint items as manual tasks in `pending` state upfront. Then it picks one, transitions it to `in_progress`, and only then creates the claude-pilot session. When done, it picks the next pending task.

This makes sprint state **observable, queryable, and recoverable** instead of living in an LLM's context window.

## Why This Approach

The current sprint mode has several problems:

1. **State is ephemeral** — sprint position lives in `current_priorities` core memory block. If the agent restarts or context compresses, sprint state can be lost or misinterpreted.
2. **No visibility** — there's no way to query "what's left in the sprint?" without asking mika-dev to recall from memory.
3. **No control surface** — cancelling, reordering, or skipping tasks requires talking to the agent instead of manipulating concrete objects.
4. **Waves are premature** — batching and parallel execution adds complexity we don't need yet. Serial execution with explicit task objects is the right foundation.

The tasks engine already supports everything needed: `manual` trigger type, status state machine (`pending` → `in_progress` → `completed`/`failed`/`cancelled`), work item patterns, and callback lifecycle for claude-pilot completion.

## Key Decisions

### Sprint initiation: Vincent provides issue list
- Vincent tells mika-dev: "sprint mika#101 mika#102 mika#103" (max 5 tickets)
- mika-dev creates one manual task per issue, all in `pending` state
- mika-dev stores the task IDs in core memory (lightweight — just IDs, not full sprint state)
- **Follow-up ticket:** mika-dev auto-selects from backlog based on priority/labels

### Execution: serial, one at a time
- mika-dev picks the first pending task, transitions to `in_progress`
- Creates claude-pilot session at this point (not before)
- Waits for callback (existing pattern via `mika ask --task-id <id> --task-complete`)
- On completion: mark task `completed`, pick next pending task
- On failure: mark task `failed`, continue to next pending task (Vincent reviews failures later)
- No waves, no parallelism — one session at a time

### Sprint grouping: independent tasks, no sprint entity
- Tasks are plain manual tasks — no `sprint_id` or parent-child relationship
- mika-dev tracks which task IDs belong to the current sprint in core memory
- This keeps the schema unchanged and avoids premature abstraction
- **Follow-up ticket:** introduce sprint grouping (parent task or tag) once the independent approach is stable

### Cancellation: both bulk and individual
- "stop sprint" command cancels all remaining pending tasks in the sprint
- Individual tasks can also be cancelled/skipped independently
- Currently running task finishes naturally (no kill signal to claude-pilot)

### Max sprint size: 5 tickets
- Hard cap at 5 to keep sprints focused and completable
- mika-dev validates this when creating sprint tasks

## What Changes

### self-dev skill prompt (`mika-skills/self-dev/`)
- Sprint mode section rewritten: instead of looping in-agent, create tasks upfront
- Sprint start: parse issue list → create manual tasks → store IDs in core memory
- Sprint loop: after each callback, check for remaining pending tasks → pick next
- Sprint stop: cancel all pending sprint tasks
- Remove wave-related logic

### mika-dev tools (already exist, may need adjustments)
- `create_work_item` — used to create sprint tasks (already exists)
- `update_work_item_status` — used to transition states (already exists)
- `list_work_items` — used to find remaining pending tasks (already exists)
- No new tools needed — the task engine API is sufficient

### No engine changes expected
- The tasks engine already supports manual tasks with the right status transitions
- No schema changes, no new trigger types, no new action types

## Open Questions

None — all questions resolved during brainstorm.

## Follow-Up Tickets

1. **Backlog-driven sprint selection** — mika-dev evaluates open issues and proposes a sprint based on priority/labels. Vincent approves before execution starts.
2. **Sprint grouping** — introduce a sprint entity (parent task or `sprint_id` tag) so sprint tasks are queryable as a group without relying on core memory. Enables sprint history and reporting.
3. **Dependent ticket ordering** — tickets that depend on each other (e.g., must be merged and compiled before the next starts). Requires the engine to enforce ordering constraints. Only pursue once independent-ticket sprints are stable.

## Scope Boundaries

- **In scope:** task creation, serial execution, failure handling, cancellation, self-dev prompt changes
- **Out of scope:** waves/parallelism, dependency ordering, backlog auto-selection, sprint entity/grouping, sprint reporting/history
