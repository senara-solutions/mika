---
title: "Investigation Panel Provider Abstraction"
category: architecture-patterns
date: 2026-03-21
tags: [llm, provider-abstraction, investigation-panel, dyn-trait, bug-fix]
related_issues: ["#224", "#71"]
---

# Investigation Panel Provider Abstraction

## Problem

The investigation panel (`POST /api/v1/investigate`) failed with "Claude API error:
Authentication failed" when a non-Anthropic LLM provider was configured. With
`MIKA_LLM_MODEL=minimax/MiniMax-M2.5` and a MiniMax API key, the investigation panel
tried to use the MiniMax key as an Anthropic key because it created its own `ClaudeClient`
directly instead of using the configured `dyn LlmProvider` from `AppState`.

This was the **last remaining call site** using `ClaudeClient` directly — all other
consumers (agent loop, compaction, team engine, task dispatcher, CLI, server handlers)
had been migrated to `dyn LlmProvider` in #71.

## Root Cause

When the multi-provider LLM abstraction was introduced (#71), the investigation panel
was intentionally kept Anthropic-only as documented in `multi-provider-llm-trait-abstraction.md`:
"Investigation panel: Kept Anthropic-only (own mini agent loop). Creates fresh `ClaudeClient`
since it's a specialized diagnostic feature." This was a reasonable decision at the time,
but became a bug once users started configuring non-Anthropic providers.

The hardcoded path in `handle_investigate()`:
```rust
let claude = ClaudeClient::new(
    state.settings.llm_api_key.clone(),
    state.settings.llm_model.clone(),
    4096,
)?;
```

## Solution

Replaced `ClaudeClient` with `state.llm` (`Arc<dyn LlmProvider>`) and migrated all
Anthropic-specific types to provider-agnostic equivalents in `investigate.rs`:

| Before (Anthropic) | After (Provider-agnostic) |
|---|---|
| `ClaudeClient::new(...)` | `state.llm.clone()` |
| `claude: &ClaudeClient` | `llm: &dyn LlmProvider` |
| `MessagesRequest` | `LlmRequest` |
| `Message` / `MessageContent` | `LlmMessage` / `LlmContent` |
| `ContentBlock::Text/ToolUse` | `LlmResponseContent::Text/ToolCall` |
| `StopReason` | `LlmStopReason` |
| `ContentBlock::ToolResult` | `LlmContentBlock::ToolResult` |
| `role: "user"` (tool results) | `LlmRole::Tool` |

Key implementation details:
- `max_tokens: 4096` hardcoded in `LlmRequest` (preserves investigation-specific budget)
- Tool definitions converted once before the loop via `.into()` (`ToolDefinition` → `LlmToolDefinition`)
- Assistant message re-injection uses `response_content_to_blocks()` helper
- `is_error` field simplified from `Option<bool>` to `bool` (provider layer handles per-provider mapping)
- Error message changed from "Claude API error" to "LLM error"
- Investigation isolation preserved: separate lock, 5-step max, 120s timeout, read-only tools

## Prevention

- All LLM-calling code paths now use `dyn LlmProvider` — no exceptions remain.
- The `multi-provider-llm-trait-abstraction.md` solution doc was updated to remove the
  "investigation is the exception" guidance.
- When adding new LLM-dependent features, always use `state.llm` / `dyn LlmProvider`,
  never `ClaudeClient` directly.
- The `ToolDefinition` type still lives in `mika_common::claude` (used by the `Tool` trait);
  convert via `.into()` when building `LlmRequest`.
