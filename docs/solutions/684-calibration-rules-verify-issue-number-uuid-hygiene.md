---
module: skills/bundled/self-dev
tags: [calibration-rules, fabrication, prompt-engineering, issue-numbers, uuid]
problem_type: prompt-enforcement
date: 2026-04-20
issue: 684
---

# Calibration Rules: Verify Issue Numbers and UUID Hygiene

## Problem

Two fabrication patterns observed in the same session (2026-04-20):

1. **Wrong issue number in completion claim:** Agent said "mika#675 complete" when the completed issue was mika#682. Similar numbers confused the agent when relying on memory.

2. **Stale task UUID:** Agent tried `check_task` with a memorized UUID that had drifted — first 8 chars correct, remaining suffix fabricated. The engine's dedup guard caught the mismatch.

Both are the same failure class: trusting memorized references instead of verifying against live tool output.

## Root Cause

LLM agents are prone to "memory fabrication" — recalling approximate values that look plausible but are incorrect. This is especially dangerous for:
- Issue numbers that differ by small amounts (#675 vs #682)
- UUIDs where the prefix is memorable but the full 36-char value drifts

## Solution

Added two calibration rules to `skills/bundled/self-dev/system_prompt.md`:

**Rule 10 — Verify issue numbers before completion claims**
- Never cite issue numbers from memory in completion messages
- Must call `list_tasks` + `check_task` and extract `reference_url`/`label` to confirm
- Cross-references Rule 8 (PR numbers) and Rule 11 (UUID lookup)

**Rule 11 — Never memorize task UUIDs**
- Store human-readable issue references (e.g., `mika#677`) in core memory, never UUIDs
- Look up UUIDs fresh from `list_tasks` filtered by `reference_url` every time
- UUIDs drift across sessions and compaction

## Pattern

This follows the established calibration rule pattern:
1. Observe a specific failure in production
2. Document the incident with exact details (date, wrong value, correct value)
3. Add a numbered rule with clear verification steps
4. Cross-reference related rules to form a coherent verification discipline

## Key Insight

Calibration rules work best when they prescribe a **verification action** (call tool X, check field Y) rather than just saying "don't do Z." The agent needs a concrete alternative behavior, not just a prohibition.
