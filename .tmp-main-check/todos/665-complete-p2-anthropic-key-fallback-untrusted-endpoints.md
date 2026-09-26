---
status: pending
priority: p2
issue_id: "665"
tags: [code-review, security]
dependencies: []
---

# Restrict Anthropic API Key Fallback for Custom Endpoints

## Problem Statement

In `Settings::make_llm_provider()`, when using a non-Anthropic provider and `llm_api_key` is not set, the code falls back to `anthropic_api_key`. For `openai-compatible/` providers with user-specified `llm_base_url`, this means a user's Anthropic API key could be sent as a Bearer token to an untrusted third-party endpoint.

## Findings

File: `crates/mika-common/src/config.rs:349-352`

```rust
_ => self
    .llm_api_key
    .clone()
    .or_else(|| self.anthropic_api_key.clone()),
```

A user who sets `MIKA_LLM_MODEL=openai-compatible/model` with `MIKA_LLM_BASE_URL=https://attacker.com/v1` but forgets `MIKA_LLM_API_KEY` would leak their Anthropic key.

## Proposed Solutions

### Option 1: No fallback for openai-compatible (Recommended)

For `ProviderKind::OpenAiCompatible`, require `llm_api_key` explicitly. Only fall back for well-known providers (OpenAi, Groq) where the key would be rejected as invalid.

**Effort:** Small.

### Option 2: Log a warning when fallback occurs

Keep fallback but log a visible warning so the user can notice.

**Effort:** Small.

## Acceptance Criteria

- [ ] `openai-compatible/model` without `llm_api_key` does NOT send `anthropic_api_key`
- [ ] `openai/gpt-4o` without `llm_api_key` can still fall back (key rejected by OpenAI anyway)
