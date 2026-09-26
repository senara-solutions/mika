---
status: complete
priority: p1
issue_id: "340"
tags: [code-review, performance, multimodal-tool-results]
dependencies: []
---

# Unbounded Memory Accumulation of Base64 Image Data Across Agent Loop Turns

## Problem Statement

When a tool returns images, the base64-encoded image data (up to ~6.7MB per image after encoding, max 5 images = ~33MB) is stored in `ToolResultBody::Blocks` within the conversation message history. On subsequent agent loop turns, this data is re-sent to the Claude API in the messages array, accumulating across turns.

Over a 10-turn agent loop with multiple image-producing tool calls, this can lead to significant memory pressure and unnecessarily large API payloads, since Claude only needs to see images on the turn they were produced.

## Findings

- **Source:** performance-oracle review agent
- **Severity:** CRITICAL — unbounded memory growth per agent loop iteration
- **Location:** `crates/mika-agent/src/agent.rs` — `process_tool_calls()` and the agent loop that accumulates `messages`
- **Evidence:** Images are stored as `ToolResultBlock::Image` in the messages vec. Each turn appends new messages without stripping image blocks from prior turns.

## Proposed Solutions

### Solution A: Strip images from prior-turn tool results before sending (Recommended)

Before building the API request, iterate over messages from prior turns and replace `ToolResultBody::Blocks` containing images with `ToolResultBody::Text` that includes a placeholder like `[image previously shown]`. Keep the current turn's images intact.

- **Pros:** Simple, effective, preserves text context, reduces memory and API payload
- **Cons:** Claude loses ability to reference prior-turn images (acceptable — it already described them in its response)
- **Effort:** Small
- **Risk:** Low

### Solution B: Move image stripping to compaction

Only strip images during conversation compaction rather than per-turn.

- **Pros:** Simpler implementation
- **Cons:** Doesn't address within-loop memory accumulation (10 turns before compaction triggers)
- **Effort:** Small
- **Risk:** Medium — doesn't solve the core problem within a single agent loop run

## Recommended Action

Solution A — strip prior-turn images before API calls

## Technical Details

- **Affected files:** `crates/mika-agent/src/agent.rs`
- **Components:** Agent loop message building, possibly `build_messages()` or a new `strip_prior_images()` helper

## Acceptance Criteria

- [ ] Images from prior agent loop turns are not re-sent to the Claude API
- [ ] Current turn's images are sent correctly
- [ ] Text from prior-turn tool results is preserved
- [ ] All existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Identified by performance-oracle agent |

## Resources

- PR branch: `feat/multimodal-tool-results`
- Claude API pricing: images consume vision tokens, resending wastes budget
