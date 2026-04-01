---
title: "Block verdict classification and auto-retry for fixable CI failures"
category: prompt-engineering
date: 2026-04-02
tags: [qa-review, self-dev, block-verdict, auto-retry, ci-failure, skill-prompt]
related_issues: ["#377", "#375"]
---

# Block verdict classification and auto-retry for fixable CI failures

## Problem

When mika-qa returned a `block` verdict for a fixable CI failure (e.g., docs-sync, clippy, fmt), mika-dev treated all blocks identically -- escalating to the user and pausing the sprint. Many CI failures are mechanically fixable (run a script, commit, push) but required human intervention because the block handler had no classification logic.

This was compounded by #375 where callback turns had insufficient step budget (10 steps) to even process the block verdict, let alone retry.

## Root Cause

The QA review skill emitted a single untyped `block` verdict for all hard failures -- CI failures, security issues, and pipeline violations were indistinguishable. The self-dev skill's block handler was a monolithic escalation path with no classification or retry capability, unlike the `hold[review]` handler which already had auto-retry.

## Solution

**Two-layer approach across two skill prompts:**

1. **QA review skill** (`qa-review/system_prompt.md`): Added typed block sub-verdicts following the existing `hold[sub_type]` pattern:
   - `block[ci]` -- CI check failures (fixable by claude-pilot)
   - `block[security]` -- Security issues in diff (non-fixable, escalate)
   - `block[pipeline]` -- Pipeline compliance failures (non-fixable, escalate)

2. **Self-dev skill** (`self-dev/system_prompt.md`): Added block classification and auto-retry:
   - Verdict parsing extended to handle `block[<sub_type>]` (same regex pattern as `hold[<sub_type>]`)
   - `block[ci]` triggers auto-retry via claude-pilot dispatch, capped at 2 retries
   - `block_retry_count` tracked in work item metadata (independent of `qa_retry_count`)
   - Metadata persistence verified after each update (infinite loop guard)
   - Non-fixable blocks (`block[security]`, `block[pipeline]`, bare `block`) retain escalation behavior

**Key design decisions:**
- Block retry budget (`block_retry_count`) is independent of hold retry budget (`qa_retry_count`) -- a PR could exhaust both across different QA cycles
- Backward compatible: bare `block` (from older QA versions) routes to non-fixable escalation
- Re-QA after fix can produce any verdict type (pass, hold, different block sub-type) -- all route correctly through existing handlers
- Sprint pauses only on non-fixable blocks or after retry exhaustion, not during active retry

## Prevention

- **Pattern: sub-typed verdicts.** When adding new verdict types to QA review, always use sub-types (e.g., `verdict[sub_type]`) rather than freeform reason parsing. Sub-types are reliable to parse; reason text is fragile.
- **Pattern: independent retry budgets.** When adding retry logic for a new verdict type, use a separate counter (e.g., `block_retry_count`) rather than sharing existing counters. This prevents unintended budget interactions across different failure modes.
- **Pattern: metadata persistence verification.** Always call `check_work_item` after `update_work_item_status` to verify metadata was persisted. Without this guard, a persistence failure makes the retry budget unbounded.
