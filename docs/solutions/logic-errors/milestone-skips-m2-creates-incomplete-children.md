---
title: "Milestone workflow skips M2, creates incomplete children"
category: logic-errors
date: 2026-04-18
tags: [self-dev, milestone, workflow, memory, qa-review]
related_issues: ["mika#638", "mika#608", "mika#617"]
related_docs: ["milestone-callback-misrouted-to-generic-workflow.md"]
---

## Problem

When mika-dev received "implement milestone mika#7", she created the milestone parent task (M1) but skipped Step M2 (fetching all milestone issues) and created only one child task for #617, immediately dispatching claude-pilot. The other 4 issues (#608, #596, #254, #609) were never tracked as children. The milestone task was stuck at `pending` with zero active children.

A second attempt recreated the milestone with correct fields but repeated the same pattern — one child, immediate dispatch. Subsequent duplicate cleanup cancelled the child and left the milestone orphaned.

Additionally, mika-dev announced "PR #640 ready" for #608 while claude-pilot was still running — the PR did not exist. This was a Rule 8 violation (fabricated PR number).

A related finding: mika-qa had zero archival memory entries from dozens of completed PR reviews, making cross-PR pattern tracking impossible.

## Root Cause

**Single-issue pattern override.** The agent's deeply trained "see issue → track → dispatch" single-issue workflow overrode the milestone's structured M1→M2→M3→M4 batch workflow. The word "milestone" didn't trigger the enumeration-first mental model. The prompt described the steps correctly but lacked enforcement gates and concrete incident references to anchor the behavior.

**No memory recording.** Neither self-dev nor qa-review skills instructed agents to call `store_fact` after task completion or PR reviews, so operational decisions were invisible across sessions.

**Premature status announcement.** After `run_claude_pilot` returned "task submitted", the agent conflated "running" with "done" and fabricated a PR number to report progress.

## Solution

### self-dev/system_prompt.md

1. Added CRITICAL gate warning before M1 with incident reference:
   > Steps M1 → M2 → M3 are setup. ALL THREE must complete before ANY dispatch.

2. Made M2 header explicit: "MANDATORY — do NOT skip"

3. Added GATE check after M2: if `milestone_issues` is empty, stop. If M2 was not run, run it now.

4. Made M3 header explicit: "MANDATORY — every issue, not just one"

5. Added GATE check after M3: `len(child_wis) == len(milestone_issues)` must be true before proceeding.

6. Added `store_fact(category="event")` at 4 milestone lifecycle points: M3 init, child complete, child blocked/failed, M5 completion.

7. Expanded Rule 8 to cover status notifications: only valid post-dispatch message is "awaiting callback", never PR numbers or "ready" claims until callback confirms.

### qa-review/system_prompt.md

8. Added "Record to memory" section: `store_fact(category="event")` after every PR review, plus `store_fact(category="preference")` for recurring patterns across 2+ PRs.

## Prevention

- **Structural gates over prose instructions.** LLMs skip prose steps when a stronger pattern (single-issue dispatch) is available. Gates with explicit verification conditions ("verify count matches") are harder to skip.
- **Incident references in prompts.** Concrete incidents with dates and task IDs anchor the instruction — the agent can pattern-match "I've seen this failure before" rather than treating the instruction as generic guidance.
- **Memory recording at lifecycle boundaries.** Without `store_fact`, operational decisions are invisible across sessions. Every workflow completion point should persist an event fact.
- **Never announce results before callbacks.** Status notifications must wait for confirmed data from tool output, not infer from dispatch success.
