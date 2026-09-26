---
status: complete
priority: p2
issue_id: 347
tags: [code-review, performance, memory]
dependencies: []
---

# strip_prior_images Should Also Strip User-Attached Images

## Problem Statement

`strip_prior_images()` only strips `ToolResult` image blocks from prior turns. User-attached `ContentBlock::Image` blocks from the initial message persist across all tool steps in the agent loop, causing the same base64 data to be re-sent on every API call within that loop.

## Findings

- **Source:** security-sentinel, agent-native-reviewer
- **Location:** `crates/mika-agent/src/agent.rs:222-257`
- **Evidence:** Only `ContentBlock::ToolResult` blocks are processed

## Proposed Solutions

### Option A: Extend strip_prior_images to also handle ContentBlock::Image (Recommended)
Add a second pass that replaces `ContentBlock::Image` with `ContentBlock::Text { text: "[user image from previous turn omitted]" }`.
- Pros: Reduces token cost for multi-step tool loops with user images
- Cons: Slightly more complex function
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] User-attached images stripped from prior messages during tool loop
- [x] Replacement text indicates images were present

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review | Pending |
| 2026-02-28 | Implemented Option A: extended strip_prior_images to handle ContentBlock::Image | Complete |
