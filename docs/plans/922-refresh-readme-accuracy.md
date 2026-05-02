# Plan: Refresh README for accuracy (#922)

## Problem

`README.md` has multiple factual drifts vs current state:
1. Claims "~1169 tests" — actual count is ~3512 (from `cargo test` output)
2. Claims "Claude (Sonnet 4.6 default) via direct API" — engine is multi-provider via `LlmProvider` trait with 11 providers
3. Project structure section omits: `mika-a2a` crate, `dashboard/`, `packages/ui/`, `skills/bundled/`
4. No mention of the Knowledge Graph subsystem

## Changes

### 1. Tech Stack table (line 76)
- Replace "Claude (Sonnet 4.6 default) via direct API" with accurate multi-provider description naming the `LlmProvider` trait and listing providers

### 2. Project Structure section (lines 85-91)
- Add `mika-a2a/` crate
- Add `dashboard/` directory
- Add `packages/ui/` directory
- Add `skills/bundled/` directory

### 3. Development section (line 112)
- Update test count from ~1169 to ~3500 (sourced from `cargo test` output: 3512 passed)

### 4. Features section
- Add Knowledge Graph bullet point with brief description

### 5. Tools table
- Verify tool count accuracy (will check during implementation)

## Verification

- Test count: `cargo test` output → 3512 passed
- Providers: `ProviderKind` enum in `crates/mika-common/src/llm/mod.rs` → 11 variants (Anthropic, OpenAI, OpenRouter, Groq, Ollama, Mistral, Google, DeepSeek, Kimi, MiniMax, Qwen)
- Crates: `ls crates/` → mika-a2a, mika-agent, mika-cli, mika-common, mika-gateway
- Top-level dirs: `ls` → dashboard/, packages/ui/, skills/bundled/ all present
