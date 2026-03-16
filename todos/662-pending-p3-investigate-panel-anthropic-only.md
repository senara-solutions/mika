---
status: pending
priority: p3
issue_id: "662"
tags: [code-review, architecture, llm-provider]
dependencies: []
---

# Investigation panel is Anthropic-only

## Problem Statement

`crates/mika-agent/src/server/investigate.rs` creates a fresh `ClaudeClient` directly rather than using `LlmProvider`. This is intentional for now (investigation is a specialized mini agent loop), but should be tracked for future multi-provider parity.

## Findings

- `investigate.rs` creates `ClaudeClient::new()` from `state.settings.anthropic_api_key`
- Uses Anthropic-specific types: `ContentBlock`, `Message`, `MessagesRequest`
- Would need to be converted to use `LlmProvider` trait for full multi-provider support
- Low priority — investigation panel is a read-only diagnostic feature

## Proposed Solutions

### Option 1: Keep as-is (Recommended for now)
- **Pros:** Works, investigation is secondary feature, Anthropic is best for this use case
- **Cons:** Won't work with non-Anthropic providers
- **Effort:** None
- **Risk:** Users on non-Anthropic providers can't use investigation panel

### Option 2: Convert to LlmProvider
- **Pros:** Full provider parity
- **Cons:** More work, investigation uses specific Anthropic features
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] Document that investigation panel requires Anthropic provider
- [ ] Convert to LlmProvider when needed
