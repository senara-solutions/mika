---
title: "fix: Truncate tool_calls metadata per-field instead of dropping tail entries"
type: fix
status: completed
date: 2026-03-13
issue: 115
---

# fix: Truncate tool_calls metadata per-field instead of dropping tail entries

## Overview

The `tool_calls` metadata on assistant messages is serialized as JSON with a 4000-char cap (`TOOL_METADATA_MAX`). When exceeded, `tool_calls_metadata_json()` drops entries from the tail — meaning later tool call records silently vanish. The dashboard needs every entry to answer "What did the agent do?". Content fields are secondary and can be truncated.

## Problem Statement

- `process_tool_calls()` truncates `input_summary` and `output_summary` to 500 chars each (agent.rs:957, agent.rs:974)
- `tool_calls_metadata_json()` serializes all summaries, and if over 4000 bytes, drops entries from the end (agent.rs:196-204)
- With 10 tool steps, worst case is ~650+ bytes/entry (500+500 chars + JSON overhead + escaping), totaling ~6500+ bytes — well over the 4000 cap
- Result: later tool calls are silently dropped, making observability incomplete

## Proposed Solution

### 1. Reduce per-field truncation limits in `process_tool_calls()`

**File:** `crates/mika-agent/src/agent.rs`

Replace the hardcoded `500` limits with named constants:

```rust
/// Maximum characters for tool input summary in metadata.
const INPUT_SUMMARY_MAX: usize = 200;
/// Maximum characters for tool output summary in metadata.
const OUTPUT_SUMMARY_MAX: usize = 300;
```

Change line 957: `truncate_summary(&input.to_string(), 500)` → `truncate_summary(&input.to_string(), INPUT_SUMMARY_MAX)`

Change lines 969-974: all `truncate_summary(..., 500)` → use `OUTPUT_SUMMARY_MAX`

### 2. Add `tracing::warn!` safety net in `tool_calls_metadata_json()`

**File:** `crates/mika-agent/src/agent.rs:196-204`

Before the tail-drop loop, add a warning log:

```rust
tracing::warn!(
    total_entries = summaries.len(),
    serialized_len = json.len(),
    max = TOOL_METADATA_MAX,
    "tool_calls metadata exceeds cap, dropping tail entries"
);
```

This should never fire under normal 10-step conditions with the new limits, but serves as an alert if it does.

### 3. Update existing tests

**File:** `crates/mika-agent/src/agent.rs` (tests section ~line 2094)

- `test_tool_calls_metadata_json_respects_max_size`: Update to use strings longer than the new limits to actually exercise truncation behavior — verify all entries are preserved
- `test_tool_call_summary_truncates_large_inputs`: Update assertions from `<= 503` to match new constants (`<= INPUT_SUMMARY_MAX` and `<= OUTPUT_SUMMARY_MAX`)
- Add new test: `test_all_entries_preserved_at_max_steps` — create 10 entries with max-length summaries, verify all 10 entries appear in the serialized output
- Add new test: `test_safety_net_warns_on_overflow` — verify the tail-drop path still works correctly as a fallback (e.g., with pathologically long tool names or extreme JSON escaping)

## Acceptance Criteria

- [x] `input_summary` truncated to 200 chars (named constant `INPUT_SUMMARY_MAX`)
- [x] `output_summary` truncated to 300 chars (named constant `OUTPUT_SUMMARY_MAX`)
- [x] `tracing::warn!` emitted when the tail-drop safety net activates in `tool_calls_metadata_json()`
- [x] All 10 tool call entries preserved in normal mode for typical tool names and content
- [x] Existing tests updated and passing with new limits
- [x] New test verifying all entries preserved at 10 steps
- [x] Dashboard frontend continues to work (no field renames — backward compatible)
- [x] `cargo test` passes, `cargo clippy` clean

## Technical Considerations

**Arithmetic validation:** With 200+300 char fields and ~150 bytes overhead per entry (JSON keys, punctuation, name, step), worst case is ~650 bytes/entry. For 10 entries: ~6500 bytes. With JSON escaping of input (which is often JSON itself), this can inflate. The safety net tail-drop remains necessary for edge cases. The key improvement is that *typical* turns (short tool names, minimal escaping) will preserve all entries, whereas before they routinely lost tail entries.

**Team mode (20 steps):** `MAX_TEAM_TOOL_STEPS = 20` means team turns will still hit the safety net. This is acceptable — the `tracing::warn!` will surface it. A future enhancement could raise `TOOL_METADATA_MAX` or use adaptive per-field limits for team mode.

**UTF-8 safety:** The existing `truncate_summary()` helper already handles char boundaries correctly. No changes needed there.

**Backward compatibility:** Old stored metadata with 500-char fields is read fine by both `format_tool_summary_block()` (re-truncates to 60/80) and dashboard `parseToolCalls()` (defensive try/catch). No migration needed.

## Files to Modify

| File | Change |
|---|---|
| `crates/mika-agent/src/agent.rs:162` | Add `INPUT_SUMMARY_MAX` and `OUTPUT_SUMMARY_MAX` constants |
| `crates/mika-agent/src/agent.rs:187-206` | Add `tracing::warn!` before tail-drop loop |
| `crates/mika-agent/src/agent.rs:957` | Use `INPUT_SUMMARY_MAX` instead of `500` |
| `crates/mika-agent/src/agent.rs:969-974` | Use `OUTPUT_SUMMARY_MAX` instead of `500` |
| `crates/mika-agent/src/agent.rs:2094+` | Update and add tests |

## Sources

- GitHub Issue: #115
- Related todo: `todos/311-complete-p2-metadata-max-size-not-enforced.md`
- Past learning: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — documents the prior removal of per-field truncation and current tail-drop behavior
- Past learning: `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md` — UTF-8 truncation panic (P1), reinforces use of boundary-safe truncation
