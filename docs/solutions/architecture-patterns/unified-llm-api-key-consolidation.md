---
title: Unified LLM API Key Consolidation
category: architecture-patterns
date: 2026-03-17
status: superseded
superseded_by: "Per-provider LLM config (#317) — MIKA_LLM_API_KEY deprecated, per-provider keys (MIKA_ANTHROPIC_API_KEY, etc.) are canonical"
tags: [config, env-vars, breaking-change, multi-provider, simplification]
related_modules: [mika-common/config, mika-cli/setup, mika-cli/doctor, mika-common/validation]
---

> **Superseded:** This consolidation was reversed by the per-provider LLM config
> plan (2026-03-22). `MIKA_LLM_API_KEY` is now deprecated and ignored.
> Per-provider keys (`MIKA_ANTHROPIC_API_KEY`, `MIKA_OPENAI_API_KEY`, etc.) are
> the canonical env vars. See #317.

# Unified LLM API Key — Remove `MIKA_ANTHROPIC_API_KEY`

## Problem

The codebase had two redundant API key env vars: `MIKA_ANTHROPIC_API_KEY` (Anthropic-specific)
and `MIKA_LLM_API_KEY` (for non-Anthropic providers). Since the provider is already determined
by the `llm_model` prefix (e.g. `openai/gpt-4o`), a provider-specific key name was unnecessary
indirection. `make_llm_provider()` had a 12-line match routing keys by provider type, including
a security guard preventing Anthropic keys from leaking to `openai-compatible` endpoints.

## Root Cause

Historical design — `MIKA_ANTHROPIC_API_KEY` was the original key before multi-provider support
was added. `MIKA_LLM_API_KEY` was bolted on as an "override for non-Anthropic providers" rather
than replacing the original.

## Solution

Consolidated to a single `MIKA_LLM_API_KEY` / `llm_api_key` field. Pre-1.0, no backward
compatibility needed.

### Key changes (29 files, net -34 lines):

1. **Removed `anthropic_api_key` field** from `Settings` struct, `CONFIG_KEYS` registry,
   `get_effective_value()`, and `Debug` impl
2. **Simplified `make_llm_provider()`** — replaced 12-line provider-based key routing with
   `self.llm_api_key.clone()` for all providers
3. **Updated `validate_api_key_format()`** — added `ApiKeyFormat::ThirdParty` variant instead
   of rejecting non-Anthropic keys. `mika doctor` now shows "third-party key" for OpenAI/Groq
   keys instead of FAIL
4. **Updated all CLI commands** — `doctor`, `config`, `setup` all reference `MIKA_LLM_API_KEY`
5. **Updated all docs** — CLAUDE.md, README, getting-started (added "Option C: Non-Anthropic
   provider"), configuration (merged duplicate Settings table rows, updated security notes)

### Checklist followed

The [config-key-rename-across-layers](config-key-rename-across-layers.md) solution's prevention
checklist was used. All 9 layers verified clean:

1. `Settings` struct field (compiler-enforced)
2. `ConfigKeyInfo` registry entry
3. `get_effective_value()` match arm
4. Manual `Debug` impl redaction
5. Direct `std::env::var()` calls (doctor.rs, setup.rs — bypass Settings)
6. Test fixtures constructing `Settings` literals
7. Handler script `unset` lists (shell-exec `run.sh`)
8. `.env.example`, `CLAUDE.md`, all docs
9. CI workflow, smoke tests

## Prevention

- **Use the rename checklist** from `config-key-rename-across-layers.md` for future env var
  renames — direct `std::env::var()` calls and `#[cfg(test)]` fixtures are invisible to
  `cargo build` alone
- **Run `cargo test` not just `cargo build`** — test helpers that construct `Settings` literals
  are only compiled under `#[cfg(test)]`
- **Handler script templates** in `templates/skills/*/handlers/` are copied to user dirs at
  startup. Updated templates propagate via bundled skill re-sync, but the old `unset` in
  deployed copies won't match until re-synced (executor-level `MIKA_*` prefix scrubbing covers
  the gap)
- **Migration for existing users**: rename `MIKA_ANTHROPIC_API_KEY` to `MIKA_LLM_API_KEY` in
  `~/.mika/.env` and shell profile. No fallback code — per project convention, pre-1.0 breaking
  changes document migration steps rather than adding compatibility shims
