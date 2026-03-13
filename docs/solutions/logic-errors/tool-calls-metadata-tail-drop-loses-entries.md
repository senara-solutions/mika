---
title: Tool calls metadata drops tail entries instead of truncating fields
category: logic-errors
date: 2026-03-13
tags: [observability, metadata, serialization, agent-loop, dashboard]
module: crates/mika-agent/src/agent.rs
issue: 115
---

# Tool calls metadata drops tail entries instead of truncating fields

## Problem

The `tool_calls` metadata field on assistant messages is serialized as JSON with a 4000-char cap (`TOOL_METADATA_MAX`). When the serialized array exceeds this limit, `tool_calls_metadata_json()` dropped entries from the **tail** of the array -- meaning entire tool call records for later steps silently disappeared. With per-field limits of 500 chars each, a 10-step turn routinely exceeded the cap, causing the dashboard to show an incomplete picture of what the agent did.

## Root Cause

The per-field truncation limits in `process_tool_calls()` (500 chars for both `input_summary` and `output_summary`) were too generous relative to the 4000-byte total cap. With 10 tool steps and ~150 bytes of JSON overhead per entry, the serialized output easily reached 6500+ bytes, triggering the tail-drop loop that silently removed later entries.

## Solution

Two-phase approach in `tool_calls_metadata_json()`:

1. **Reduce initial per-field limits**: `INPUT_SUMMARY_MAX = 200`, `OUTPUT_SUMMARY_MAX = 300` (named constants replacing hardcoded `500`).
2. **Progressive re-truncation before tail-drop**: If the initial serialization exceeds the cap, aggressively re-truncate fields to 30/50 chars to preserve all entries. Only drop tail entries as a last resort (with `warn!` logging).

Key code changes in `crates/mika-agent/src/agent.rs`:
- Added `INPUT_SUMMARY_MAX` (200) and `OUTPUT_SUMMARY_MAX` (300) constants
- `process_tool_calls()` uses these constants instead of hardcoded `500`
- `tool_calls_metadata_json()` tries aggressive field truncation (30/50 chars) before falling back to tail-drop
- `warn!` logged when tail-drop activates or when no entries fit at all

## Prevention

- **Budget math before limits**: When setting per-field truncation limits, calculate worst-case total: `entries * (field_limits + overhead)` and verify it fits within the cap. With 10 entries and ~150 bytes overhead each, 4000 - 1500 = 2500 bytes for content, or ~250 per entry across both fields.
- **Prefer field truncation over entry dropping**: For observability metadata, knowing *which* tools ran (the index) is more valuable than having full content. Always truncate content fields before dropping structural entries.
- **Name the constants**: Replace magic numbers with named constants (`INPUT_SUMMARY_MAX`, `OUTPUT_SUMMARY_MAX`) so the budget relationship between per-field limits and total cap is self-documenting.
- **Log safety-net activation**: If a fallback path should "never fire under normal conditions," add a warning log so you know when it does.

## Related

- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` -- Documents the dashboard's consumption of this metadata and frontend `parseToolCalls()` behavior
- `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md` -- UTF-8 truncation panic (P1) that reinforces using boundary-safe truncation
