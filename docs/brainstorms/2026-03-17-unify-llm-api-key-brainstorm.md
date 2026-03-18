# Brainstorm: Unify LLM API Key Configuration

**Date:** 2026-03-17
**Status:** Draft

## What We're Building

Remove `MIKA_ANTHROPIC_API_KEY` and consolidate to a single `MIKA_LLM_API_KEY` env var for all LLM providers. The provider is already determined by the `llm_model` prefix (e.g., `openai/gpt-4o`, `ollama/llama3`, or bare `claude-sonnet-4-6` for Anthropic). Having a separate Anthropic-specific key is redundant and creates two sources of truth.

## Why This Approach

- **Single source of truth:** One key field, one env var. Provider routing is already handled by `llm_model`.
- **Consistent naming:** `MIKA_LLM_API_KEY` aligns with `MIKA_LLM_MODEL` and `MIKA_LLM_BASE_URL`.
- **No backward compatibility needed:** Pre-1.0, breaking changes ship without compat shims.
- **Simpler code:** The `make_llm_provider()` key resolution logic (Anthropic vs OpenAiCompatible vs fallback chain) collapses to a single field read.
- **Security simplification:** The guard preventing Anthropic key leakage to `openai-compatible/` endpoints becomes moot — there's one key, the user controls what it contains.

## Key Decisions

1. **Env var name:** `MIKA_LLM_API_KEY` (matches existing LLM config family)
2. **Settings field name:** `llm_api_key` (already exists, just becomes the only key)
3. **No deprecation/fallback:** Remove `anthropic_api_key` entirely, no migration shim
4. **Config key registry:** Remove `anthropic_api_key` from `CONFIG_KEYS`, update `llm_api_key` description to drop "override" / "non-Anthropic" language
5. **OAuth detection:** Keep the `sk-ant-oat*` prefix detection for bearer vs API key auth — it moves to wherever `llm_api_key` is consumed by the Anthropic provider

## Scope

**Files to modify (~7):**
- `crates/mika-common/src/config.rs` — Remove `anthropic_api_key` field, update `CONFIG_KEYS`, simplify `make_llm_provider()`, update `Debug` impl
- `crates/mika-cli/src/commands/doctor.rs` — Check `MIKA_LLM_API_KEY` instead
- `crates/mika-cli/src/commands/config.rs` — Update auth display
- `crates/mika-cli/src/commands/setup.rs` — Prompt for `MIKA_LLM_API_KEY`
- `crates/mika-agent/src/server/investigate.rs` — Use `llm_api_key`
- `.env.example` — Update variable name and comments
- Docs + CLAUDE.md — Update references

**Test files:** `test_utils.rs`, `server/mod.rs` test fixtures

## Open Questions

None — scope is well-defined.
