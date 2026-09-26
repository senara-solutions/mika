---
status: complete
priority: p2
issue_id: "311"
tags: [code-review, correctness]
dependencies: ["310"]
---

# TOOL_METADATA_MAX cap not enforced after second-pass truncation

## Problem Statement

`tool_calls_metadata_json` uses a two-pass strategy: serialize once, if over 4000 bytes re-truncate fields to 80/100 chars and re-serialize. The second-pass result is returned without checking its length. With many tool calls (10 steps x multiple tools), the fallback can still exceed 4000 bytes.

Identified by: architecture-strategist, security-sentinel, performance-oracle

## Findings

- The test `tool_calls_metadata_json_respects_max_size` asserts valid JSON but NOT that `json.len() <= TOOL_METADATA_MAX`
- With 50 entries at 80+100 chars + JSON overhead, the fallback output is ~10-16K bytes
- The DB column has no length constraint, so oversized data is silently stored
- The stated contract ("capped at TOOL_METADATA_MAX") is violated

## Proposed Solutions

### Option A: Lower initial limits so first pass always fits (Recommended)
Reduce `TOOL_INPUT_SUMMARY_MAX` to 120 and `TOOL_OUTPUT_SUMMARY_MAX` to 180. With MAX_TOOL_STEPS=10 and ~60 bytes JSON overhead per entry: `10 × (120 + 180 + 60) = 3600 < 4000`. Delete the second pass entirely.
- Pros: Simpler code, eliminates the clone+re-serialize path
- Cons: Slightly less detail in stored summaries
- Effort: Small

### Option B: Add entry-count cap after second pass
After re-truncation, drop entries from the tail until under the 4000-byte limit.
- Pros: Preserves current limits for normal cases
- Cons: More complex, keeps two-pass logic
- Effort: Small

## Technical Details

- **Affected file:** `crates/mika-agent/src/agent.rs:131-153`
- **Test to fix:** `tool_calls_metadata_json_respects_max_size` — add `assert!(json.len() <= TOOL_METADATA_MAX)`

## Acceptance Criteria

- [ ] Serialized metadata always fits within TOOL_METADATA_MAX
- [ ] Test asserts the size bound, not just structural validity
- [ ] Works with 10+ tool calls per turn

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
