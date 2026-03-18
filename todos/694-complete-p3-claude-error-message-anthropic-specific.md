---
status: pending
priority: p3
issue_id: 694
tags: [code-review, quality]
dependencies: []
---

# Update claude.rs Error Message for Multi-Provider Support

## Problem Statement

The error message in `crates/mika-common/src/claude.rs` when `MIKA_LLM_API_KEY` is missing still says "Set it to an API key (sk-ant-api03-...) or OAuth token (sk-ant-oat01-...)". Since `MIKA_LLM_API_KEY` now supports non-Anthropic keys, this is misleadingly narrow. Additionally, `ClaudeClient` is used by the investigation panel which constructs it from `settings.llm_api_key` — a user with a non-Anthropic key would see Anthropic-specific hints.

## Proposed Solutions

### Option A: Broaden the hint
- Add "or your provider's API key" to the error message
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Error message mentions non-Anthropic providers as an option

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |
