---
title: "fix: Sort milestone issues ascending in self-dev Step M2"
type: fix
status: active
date: 2026-04-17
---

# fix: Sort milestone issues ascending in self-dev Step M2

## Overview

`gh issue list` returns issues in descending created order by default. The self-dev milestone workflow (Step M2) processes issues in this default order, meaning the newest issue runs first. When child issues have dependency ordering (earlier issues land first, later ones depend on them), this causes dependency breakage. Fix by adding `sort_by(.number)` to the jq filter so issues are processed oldest-first.

## Problem Frame

Caught when milestone #12 was about to be dispatched: #630 depends on #629, but the current ordering would queue #630 first and break it. The root cause is that `gh issue list` returns newest-first by default, and the Step M2 jq filter (`.[].number`) passes through that order without sorting.

The same pattern exists in the Project Workflow (Step P2), where GraphQL returns items in project-board order (non-deterministic). Both should sort ascending by issue number for deterministic, dependency-safe ordering.

## Requirements Trace

- R1. Step M2 must return issues in ascending number order (from issue #632 acceptance criteria)
- R2. Plan captures reasoning: newest-first is wrong for dependency-ordered work
- R3. Compound doc surfaces the general pattern: batch work-item fetches should sort deterministically, oldest-first

## Scope Boundaries

- Prompt-only edit to `skills/bundled/self-dev/system_prompt.md`
- No Rust code changes
- No test changes (prompt text, not executable code)

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` line 337: Step M2 jq filter `'.[].number'`
- `skills/bundled/self-dev/system_prompt.md` line 438: Step P2 jq filter (GraphQL, same lack of sorting)

## Key Technical Decisions

- **Sort by `.number` ascending:** Issue numbers are monotonically increasing and match creation order. This is stable, deterministic, and respects natural dependency ordering where earlier issues are prerequisites for later ones.
- **Also fix Step P2:** The Project Workflow has the same problem — GraphQL returns items in project-board order, which is non-deterministic. Apply the same `sort_by(.number)` pattern for consistency.

## Implementation Units

- [x] **Unit 1: Add sort_by(.number) to Step M2 and Step P2 jq filters**

**Goal:** Ensure milestone and project issue lists are returned in ascending issue number order.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Step M2 (line 337): Change jq filter from `'.[].number'` to `'sort_by(.number) | .[].number'`
- Step P2 (line 438): The GraphQL jq filter outputs `repo#number` strings. Wrap with a `sort_by` on the numeric suffix: pipe the output through `sort_by(split("#")[1] | tonumber)` or restructure to sort before formatting. The simplest approach: collect into array, sort_by number, then emit.

**Patterns to follow:**
- jq `sort_by()` is the standard approach for deterministic ordering in gh CLI pipelines

**Test expectation:** none — this is a prompt text edit, not executable code. Verification is by reading the updated command.

**Verification:**
- The Step M2 command in system_prompt.md includes `sort_by(.number)` before `.[].number`
- The Step P2 command in system_prompt.md sorts items by issue number ascending
- Reading the jq filters confirms ascending order output

## System-Wide Impact

- **Interaction graph:** Step M3 (create child work items) and Step M4 (serial execution loop) consume the ordered list from M2/P2. Sorting ascending means dependencies are naturally satisfied as the loop processes items sequentially.
- **Unchanged invariants:** The Step M2/P2 output format is unchanged (list of issue numbers / repo#number strings). Only the ordering changes.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| jq `sort_by` syntax error in prompt | Simple, well-documented jq operation; verify by reading |

## Sources & References

- Related issue: #632
- Related incident: #630 depends on #629, milestone #12 would have broken
