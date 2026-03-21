---
title: "feat: Add first-class provider prefixes for MiniMax, Qwen, and Kimi"
type: feat
status: completed
date: 2026-03-21
origin: docs/brainstorms/2026-03-18-openai-migration-brainstorm.md (on branch docs/openai-migration-brainstorm)
---

# feat: Add first-class provider prefixes for MiniMax, Qwen, and Kimi

## Overview

After Anthropic blocked OAuth tokens (`sk-ant-oat*`) for third-party apps on Jan 9 2026, Mika users need cost-effective alternative LLM providers. The multi-provider infrastructure already exists (`LlmProvider` trait, `OpenAiCompatibleProvider`, `ModelSpec::parse()`). This adds first-class `minimax/`, `qwen/`, and `kimi/` prefixes with built-in default base URLs so users don't need the generic `openai-compatible/` prefix + manual `MIKA_LLM_BASE_URL` configuration.

## Acceptance Criteria

- [x] `MIKA_LLM_MODEL=minimax/MiniMax-M2.5` works without setting `MIKA_LLM_BASE_URL`
- [x] `MIKA_LLM_MODEL=qwen/qwen3.5-plus` works without setting `MIKA_LLM_BASE_URL`
- [x] `MIKA_LLM_MODEL=kimi/kimi-k2.5` works without setting `MIKA_LLM_BASE_URL`
- [x] All three providers report `supports_vision() -> true`
- [x] `mika doctor` validates connectivity for the configured provider
- [x] `.env.example` documents the new prefixes with example configs
- [x] `docs/configuration.md` updated with provider migration guide
- [x] `CLAUDE.md` Stack section updated with new provider prefixes
- [x] Unknown prefix error message lists the new providers
- [x] Existing `openai-compatible/` path still works (no regression)
- [x] All tests pass (`cargo test`)

## Implementation

### 1. Add `ProviderKind` variants

**File:** `crates/mika-common/src/llm/mod.rs`

Add three new variants to the `ProviderKind` enum (line ~74):

```rust
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    Groq,
    MiniMax,        // NEW
    Qwen,           // NEW
    Kimi,           // NEW
    OpenAiCompatible,
}
```

### 2. Add prefix matching in `ModelSpec::parse()`

**File:** `crates/mika-common/src/llm/mod.rs` (line ~116)

Add to the match arm:

```rust
"minimax" => ProviderKind::MiniMax,
"qwen" => ProviderKind::Qwen,
"kimi" => ProviderKind::Kimi,
```

### 3. Add default base URLs

**File:** `crates/mika-common/src/llm/mod.rs` (line ~154)

```rust
ProviderKind::MiniMax => Some("https://api.minimax.chat/v1"),
ProviderKind::Qwen => Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
ProviderKind::Kimi => Some("https://api.moonshot.ai/v1"),
```

### 4. Add dispatch in `create_provider()`

**File:** `crates/mika-common/src/llm/mod.rs` (line ~176)

Add `MiniMax | Qwen | Kimi` to the same match arm as `OpenAi | Ollama | Groq` — they all create `OpenAiCompatibleProvider`. No special handling needed.

### 5. Enable vision support

**File:** `crates/mika-common/src/llm/openai.rs` (line ~290)

Update `supports_vision()` to return `true` for the new providers:

```rust
fn supports_vision(&self) -> bool {
    matches!(self.provider_kind, ProviderKind::OpenAi | ProviderKind::MiniMax | ProviderKind::Qwen | ProviderKind::Kimi)
}
```

### 6. Update error message

**File:** `crates/mika-common/src/llm/mod.rs` — the unknown-prefix error (line ~127)

Add `minimax`, `qwen`, `kimi` to the list of known prefixes in the error message.

### 7. Add CLI model aliases (optional convenience)

**File:** `crates/mika-cli/src/cli.rs` (line ~536)

```rust
("minimax", "minimax/MiniMax-M2.5"),
("qwen", "qwen/qwen3.5-plus"),
("kimi", "kimi/kimi-k2.5"),
```

### 8. Update `.env.example`

Add example configurations for each provider with comments explaining which to choose.

### 9. Create `docs/configuration.md` — provider migration guide

New file documenting:
- All supported providers with prefix, default base URL, vision support
- Migration guide: switching from Anthropic to MiniMax/Qwen/Kimi
- Known limitations of non-Claude providers (no extended thinking, no prompt caching)
- Cost comparison table (from brainstorm)

### 10. Update `CLAUDE.md`

Update the Stack section's LLM line to mention the new provider prefixes.

### 11. Add/update tests

**File:** `crates/mika-common/src/llm/mod.rs` (test module)

- Test `ModelSpec::parse("minimax/MiniMax-M2.5")` → `ProviderKind::MiniMax` + model `MiniMax-M2.5`
- Test `ModelSpec::parse("qwen/qwen3.5-plus")` → `ProviderKind::Qwen` + model `qwen3.5-plus`
- Test `ModelSpec::parse("kimi/kimi-k2.5")` → `ProviderKind::Kimi` + model `kimi-k2.5`
- Test default base URLs are set correctly
- Test `openai-compatible/` still works (regression guard)

## Sources

- **Origin brainstorm:** [docs/brainstorms/2026-03-18-openai-migration-brainstorm.md](docs/brainstorms/2026-03-18-openai-migration-brainstorm.md) (on branch `docs/openai-migration-brainstorm`) — MiniMax M2.5 as primary recommendation, Qwen 3.5 Medium as fallback, Kimi K2.5 for coding tasks
- **Multi-provider architecture:** [docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md](docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md)
- **API key consolidation:** [docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md](docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md)
- Related issue: #197
