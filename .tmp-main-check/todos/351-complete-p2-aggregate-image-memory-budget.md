---
status: complete
priority: p2
issue_id: 351
tags: [code-review, security, performance, memory]
dependencies: []
---

# No Aggregate Image Memory Budget Per Agent Step

## Problem Statement

Within a single agent loop step, all tool calls are processed sequentially and their results (including base64 images) accumulate in `tool_results` before being appended to `request.messages`. The per-tool limit is 5 images at 5MB each (~33MB base64), but with up to 10 tool calls per step, worst-case peak memory could reach ~333MB of base64 strings within a single step. This occurs before `strip_prior_images` runs on the next step.

While `strip_prior_images` correctly prevents cross-step accumulation, there is no intra-step budget. In a per-customer container with tight memory limits (128-256MB), this could cause OOM.

## Findings

- **Source:** security-sentinel, performance-oracle (convergent findings)
- **Location:** `crates/mika-agent/src/agent.rs:676-750` (process_tool_calls)
- **Evidence:** No cumulative byte tracking across tool results within a step

## Proposed Solutions

### Option A: Track cumulative base64 bytes per step (Recommended)
Add a running total of base64 bytes in `process_tool_calls`. Once a threshold is reached (e.g., 20MB total base64), skip including new images and append a text note instead.
- Effort: Small
- Risk: Low

### Option B: Reduce MAX_IMAGES_PER_RESULT to 3
Reduces worst case from ~333MB to ~200MB. Simpler but less precise.
- Effort: Trivial
- Risk: Low

### Option C: Document container memory requirements
Add a comment documenting that containers should be provisioned with 256MB+ to safely handle maximum image loads.
- Effort: Trivial
- Risk: None (no code change)

## Acceptance Criteria

- [ ] Peak memory for image data within a single step is bounded
- [ ] Text fallback when budget is exceeded (images skipped with note)
- [ ] Container memory requirements documented

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review | Pending |
| 2026-02-28 | Fixed: added MAX_IMAGE_BYTES_PER_STEP (20MB) budget tracking in process_tool_calls | Complete |
