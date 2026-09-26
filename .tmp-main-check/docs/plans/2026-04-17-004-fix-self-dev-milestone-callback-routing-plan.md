---
title: "fix: Self-dev milestone callback routing + missing parent_task_id in child creation"
type: fix
status: active
date: 2026-04-17
---

# fix: Self-dev milestone callback routing + missing parent_task_id in child creation

## Overview

Two prompt gaps in self-dev's Milestone Workflow caused mika#6 (first milestone dispatch) to break. Callback turns containing child issue references get misrouted to the Generic Workflow instead of the milestone callback handler. Separately, `create_work_item` calls in Step M3 lack a JSON example showing `parent_task_id`, causing the agent to omit it.

## Problem Frame

During the mika#6 milestone run, claude-pilot completed child #1 (mika#582). The callback arrived as a `SilentTrigger::Callback` turn containing "mika#582" as an issue reference. mika-dev's LLM pattern-matched this to the Generic Workflow ("implement mika#582") instead of the Callback Entry Point, creating orphan work items and abandoning the milestone loop. Separately, all 3 children were created without `parent_task_id` because the agent follows JSON examples literally and the Step M3 section uses bullet-list format without an explicit JSON block.

## Requirements Trace

- R1. Callback turns during a milestone/project run must route to Step M4, not the Generic Workflow
- R2. `create_work_item` calls in Step M3 must include `parent_task_id` in both prose AND a JSON example block
- R3. Step P3 (Project Workflow) must receive the same JSON example treatment for consistency

## Scope Boundaries

- Prompt-only changes to `skills/bundled/self-dev/system_prompt.md`
- No Rust code changes
- No changes to other skills or handler scripts

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` — the single file being modified
- Callback Entry Point section (lines 79–98) — currently has no milestone/project awareness
- Step M3 (lines 312–319) — uses bullet-list format; `parent_task_id` is present but not in JSON
- Step P3 (lines 405–414) — same bullet-list pattern, should get JSON too for consistency
- Step 3 in main workflow (lines 46–55) — uses JSON code block for `run_claude_pilot` — this is the pattern the agent follows reliably

### Institutional Learnings

- Rule 4 (lines 225–235) documents tool input schema discipline failures — the agent skips parameters not shown in examples
- The `run_claude_pilot` call in Step 3 already uses JSON format successfully — this is the proven pattern

## Key Technical Decisions

- **Add milestone-awareness to Callback Entry Point, not a separate section:** The routing fix belongs in the existing Callback Entry Point since that's where the LLM enters on callback turns. Adding a separate section risks the same pattern-matching problem.
- **Use JSON code blocks for create_work_item in M3/P3:** The agent reliably follows JSON examples (see Step 3's `run_claude_pilot` format). Bullet lists are ambiguous — JSON is unambiguous.
- **Place milestone check BEFORE the success/failure handling:** The milestone context check must happen at the top of the Callback Entry Point, before branching into success/failure paths, so the agent knows it's in milestone mode regardless of outcome.

## Implementation Units

- [ ] **Unit 1: Add milestone/project awareness to Callback Entry Point**

**Goal:** Prevent callback turns from being misrouted to the Generic Workflow when the callback is part of a milestone or project execution loop.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Insert a new block after the "Callback Entry Point" heading (after line 81) and before the "On pipeline failure" section (line 87)
- The block should instruct the agent to check if the callback's work item has a parent with `type='milestone'` or `type='project'` via `check_work_item`
- If milestone/project context detected: after extracting metadata and updating the child work item, return to Step M4/P4 — do NOT re-read issues, create new work items, or enter the Generic Workflow
- Use strong negative instructions matching the style of existing calibration rules

**Patterns to follow:**
- Rule 9 (Webhook turns are not dispatch triggers) — same pattern of explicit "do NOT" instructions to prevent misrouting
- SCOPE RULE blocks already in the Callback Entry Point section

**Test scenarios:**
- Test expectation: none — prompt-only change, no code to test. Verified manually via milestone dispatch.

**Verification:**
- The Callback Entry Point section contains explicit milestone/project routing instructions
- The instructions reference Step M4/P4 by name
- Strong negative instructions prevent entering the Generic Workflow

- [ ] **Unit 2: Add JSON example blocks to Step M3 and Step P3**

**Goal:** Make `parent_task_id` unmissable in milestone/project child creation by providing explicit JSON examples.

**Requirements:** R2, R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Replace the bullet-list format in Step M3 with a JSON code block showing all parameters including `parent_task_id`
- Apply the same JSON code block treatment to Step P3
- Follow the same JSON format used in Step 3 for `run_claude_pilot`

**Patterns to follow:**
- Step 3's `run_claude_pilot` JSON example (lines 48–52) — proven to be followed reliably by the agent

**Test scenarios:**
- Test expectation: none — prompt-only change, no code to test. Verified manually via milestone dispatch.

**Verification:**
- Step M3 contains a JSON code block with `parent_task_id` as a visible field
- Step P3 contains a matching JSON code block with `parent_task_id`
- Both JSON blocks include all required fields: `type`, `parent_task_id`, `label`, `reference_url`, `source`

## System-Wide Impact

- **Interaction graph:** Callback turns now have an additional routing check. The milestone/project loop (M4/P4) becomes reachable from callbacks, which was the intended design but was not enforced in the prompt.
- **Unchanged invariants:** Single-issue workflow (Steps 1-6), webhook handling, sprint mode, and all calibration rules are unchanged. The Callback Entry Point's success/failure handling paths remain the same — the milestone check is an additional guard that routes before those paths when applicable.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Milestone check adds latency (extra `check_work_item` call) | Minimal — one lightweight DB lookup per callback turn, only when parent_task_id exists |
| Over-broad matching routes non-milestone callbacks to M4 | Check is gated on parent work item having `type='milestone'` or `type='project'` — regular issue callbacks have no parent |

## Sources & References

- Related issue: #609
- Incident: Milestone mika#6 run, 2026-04-16 ~22:22-23:01 UTC
- mika-dev session `93919db5` (callback turn where the break happened)
