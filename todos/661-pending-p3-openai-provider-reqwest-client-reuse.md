---
status: pending
priority: p3
issue_id: "661"
tags: [code-review, performance, llm-provider]
dependencies: []
---

# OpenAI provider creates reqwest::Client per instance

## Problem Statement

`OpenAiCompatibleProvider::new()` creates a new `reqwest::Client` each time. While this is fine for single-provider usage (one per container), if multiple providers are ever created (e.g., model switching recreates the provider), the connection pool is not shared. This is a minor concern for now since model switches are rare.

## Findings

- `crates/mika-common/src/llm/openai.rs` — `reqwest::Client::builder()` called in constructor
- Model switching in `chat.rs` calls `make_llm_provider()` which creates a new provider + new client
- `reqwest::Client` docs recommend reusing a single client for connection pooling

## Proposed Solutions

### Option 1: Accept current behavior (Recommended)
- **Pros:** Simple, model switching is rare, one provider per container lifecycle
- **Cons:** No connection pooling across model switches
- **Effort:** None
- **Risk:** Minimal

### Option 2: Share reqwest::Client via parameter
- **Pros:** Connection pool reuse
- **Cons:** More plumbing, premature optimization
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Decide if shared client is needed based on usage patterns
