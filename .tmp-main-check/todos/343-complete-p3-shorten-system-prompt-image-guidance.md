---
status: complete
priority: p3
issue_id: "343"
tags: [code-review, quality, multimodal-tool-results]
dependencies: []
---

# Shorten System Prompt Image Guidance

## Problem Statement

The two lines added to the system prompt in `crates/mika-agent/src/prompt.rs` for image guidance could be condensed into one shorter line, saving ~70 characters of prompt tokens.

## Findings

- **Source:** code-simplicity-reviewer agent
- **Severity:** P3 — minor token savings
- **Location:** `crates/mika-agent/src/prompt.rs` — Tool Usage section

## Proposed Solutions

### Solution A: Merge into single line

Combine the two lines into one concise line:
```
- Tools may return images (screenshots, image files); you will see and can describe their contents.
```

- **Pros:** Shorter, saves tokens, equally clear
- **Cons:** Slightly less detailed
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria

- [ ] Prompt is shorter while preserving meaning
- [ ] Prompt content tests updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Identified by code-simplicity-reviewer agent |
