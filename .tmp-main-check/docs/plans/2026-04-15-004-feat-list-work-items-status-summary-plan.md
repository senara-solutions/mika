---
title: "feat: list_work_items returns status-count summary + filter guidance"
type: feat
status: active
date: 2026-04-15
issue: 572
---

# feat: list_work_items returns status-count summary + filter guidance

## Overview

Add a status-count summary line and a filter guidance note to `list_work_items` tool output, so the agent sees aggregate status distribution in one call and never needs to re-query with status filters to verify counts.

## Problem Frame

Models (kimi-k2.5, qwen, others) routinely follow an unfiltered `list_work_items({})` with 3+ redundant filtered calls (`status:"in_progress"`, `status:"blocked"`, `status:"completed"`) to reconstruct counts the first response already contained. This is ritual thoroughness — the unfiltered output shows each item's status but doesn't make the aggregate obvious. The fix is structural: compute the summary server-side and include just-in-time guidance discouraging defensive re-filtering.

## Requirements Trace

- R1. Unfiltered calls include a `summary` line: `"N items total — X blocked, Y in_progress, Z pending"` (omit zero-count statuses)
- R2. Filtered calls include a scoped summary for the filtered subset
- R3. Unfiltered calls include a `note` field with filter guidance
- R4. No new DB queries — counts computed in-memory from the already-fetched result set
- R5. No new fields on `ToolContext`, no new external crates
- R6. All existing tests pass, new tests cover summary and note

## Scope Boundaries

- This change applies only to `list_work_items`. Do not generalize to `list_tasks`, `list_agents`, etc. (YAGNI — measure impact first)
- No changes to the DB layer, `Task` struct, or `list_manual_tasks` query
- No JSON serialization change — output remains plain text via `ToolOutput::success(String)`

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/list_work_items.rs` — target file, currently builds plain-text output with `"Work items (N):\n"` header + item lines
- `crates/mika-agent/src/tools/list_skills.rs` — direct precedent: `skipped_warning()` helper appends a conditional footer line after the item listing
- `crates/mika-agent/src/tools/update_work_item_status.rs` — `VALID_STATUSES` and `VALID_TRANSITIONS` constants define the 5 statuses
- `crates/mika-agent/src/db.rs` — `list_manual_tasks()` returns `Vec<(Task, Option<i64>)>`, capped at 50, already has status on each `Task`

### Institutional Learnings

- `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md` — "Splitting one logical operation across two tools introduced three independent failure modes." Embedding summary data in list responses is the correct agent-native pattern.
- `docs/solutions/logic-errors/create-work-item-duplicate-on-retry.md` — structural signals in tool output are more reliable than prompt instructions alone for preventing over-action.

## Key Technical Decisions

- **Plain-text format, not JSON:** All list tools return formatted text. The summary and note are appended as text lines, following the `list_skills` footer pattern.
- **Summary format:** `"N items total — X status_a, Y status_b"` with em-dash separator, omitting zero-count statuses. Status order: `pending`, `in_progress`, `blocked`, `completed`, `cancelled` (matches `VALID_STATUSES` declaration order, groups active before terminal).
- **Note only on unfiltered calls:** The guidance note ("All items are returned...") only makes sense when no status filter is applied. Filtered calls already demonstrate intent.
- **Summary on both filtered and unfiltered:** Filtered calls get a scoped summary (e.g., "2 items total — 2 blocked") so the output is self-describing regardless of filter.

## Implementation Units

- [x] **Unit 1: Add status-count summary and filter guidance note**

**Goal:** Compute status counts from the in-memory result set and append summary + note lines to the tool output.

**Requirements:** R1, R2, R3, R4, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/list_work_items.rs`

**Approach:**
- After fetching `items` and before building the output string, iterate `items` to build a `HashMap<&str, usize>` (or a fixed-order counting approach) of status → count
- Build a summary string: total count, then each non-zero status in declaration order (`pending`, `in_progress`, `blocked`, `completed`, `cancelled`), separated by commas, with em-dash after total
- Build the note string (constant) for unfiltered calls only
- Append summary after the item listing, then note (if applicable), separated by blank lines
- Follow the `list_skills` pattern: helper function(s) that return empty string when not applicable

**Patterns to follow:**
- `list_skills.rs` `skipped_warning()` — conditional footer helper returning `String`
- Existing `VALID_STATUSES` constant in `list_work_items.rs` for status order

**Test scenarios:**
- Happy path: unfiltered call with mixed statuses (2 pending, 1 in_progress, 1 blocked) → summary includes all three non-zero statuses in correct order, note field present
- Happy path: unfiltered call with single status (3 pending) → summary shows "3 items total — 3 pending", note present
- Happy path: filtered call (`status: "blocked"`) with 2 blocked items → summary shows "2 items total — 2 blocked", no note
- Edge case: empty result → no summary or note (existing "No work items found" message unchanged)
- Edge case: all 5 statuses represented → summary lists all 5 in declaration order
- Integration: existing tests (`test_list_work_items_basic`, `test_list_work_items_filter_by_status`) still pass with updated output format

**Verification:**
- `cargo test -p mika-agent` passes (all existing + new tests)
- `cargo clippy -p mika-agent` clean
- Output format matches acceptance criteria from issue #572

## System-Wide Impact

- **API surface parity:** No other consumers of `list_manual_tasks` are affected — the change is purely in the tool's output formatting layer
- **Unchanged invariants:** `ToolOutput` struct, `list_manual_tasks` DB method, `Task` struct, and all other list tools remain unchanged
- **Delegate visibility:** Delegates and silent agents also call `list_work_items` (read-only tool in `default_tools()`). The summary is lightweight context, not a call-to-action — appropriate for all agent types

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing test assertions break due to output format change | Review all 6 existing tests; update assertions that check exact header format if needed |

## Sources & References

- Related issue: #572
- Related code: `crates/mika-agent/src/tools/list_work_items.rs`, `crates/mika-agent/src/tools/list_skills.rs`
- Related learnings: `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md`
