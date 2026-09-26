---
title: "fix: Make /model aliases provider-aware for cross-provider correctness"
type: fix
status: active
date: 2026-04-06
---

# fix: Make /model aliases provider-aware for cross-provider correctness

## Overview

After switching to OpenRouter via `/provider openrouter`, the `/model` command shows Anthropic-centric content: the command hint says `[sonnet|opus|haiku]`, all aliases are listed regardless of provider, and using bare Anthropic aliases (`sonnet`, `opus`, `haiku`) sets invalid model names on OpenRouter.

## Problem Statement

`MODEL_ALIASES` in `cli.rs:675` has 6 entries with inconsistent formatting:
- 3 Anthropic aliases use bare model names: `("sonnet", "claude-sonnet-4-6", ...)`
- 3 cross-provider aliases include provider prefix: `("gpt4o", "openai/gpt-4o", ...)`

On OpenRouter (where `model_names_contain_slash()` returns true), `parse_provider_model()` treats ALL input as the model name without splitting on `/`. This means:
- `sonnet` → `claude-sonnet-4-6` → invalid OpenRouter model (needs `anthropic/claude-sonnet-4-6`)
- `gpt4o` → `openai/gpt-4o` → valid OpenRouter model (works by accident)

The same bug exists in the CLI `mika ask --model sonnet` path via `resolve_model_alias()`.

## Proposed Solution

**Normalize all `MODEL_ALIASES` to include provider prefix.** This is the minimal, highest-leverage fix:

```rust
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "anthropic/claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "anthropic/claude-haiku-4-5", "Claude Haiku 4.5"),
    ("gpt4o", "openai/gpt-4o", "GPT-4o"),
    ("deepseek", "deepseek/deepseek-chat", "DeepSeek Chat"),
    ("gemini", "google/gemini-2.5-flash", "Gemini 2.5 Flash"),
];
```

**Why this works on all providers:**
- On Anthropic: `parse_provider_model("anthropic/claude-sonnet-4-6", Anthropic)` → splits on `/`, parses `anthropic` as valid `ProviderKind` → returns `(Anthropic, "claude-sonnet-4-6")` ✓
- On OpenRouter: `parse_provider_model("anthropic/claude-sonnet-4-6", OpenRouter)` → `model_names_contain_slash()` is true → returns `(OpenRouter, "anthropic/claude-sonnet-4-6")` which IS a valid OpenRouter model name ✓
- Cross-provider from Anthropic: `/model gpt4o` → `parse_provider_model("openai/gpt-4o", Anthropic)` → splits → `(OpenAi, "gpt-4o")` ✓ (unchanged)

Additionally:
- Update `/model` `args_hint` from `[sonnet|opus|haiku]` to `[<name|alias>]`
- Fix `format_display_model` interaction: the "Already using" check in `switch_model()` needs to handle the new prefixed format

## Technical Considerations

### Files to change

1. **`crates/mika-cli/src/cli.rs:675`** — Normalize `MODEL_ALIASES` entries to include `anthropic/` prefix
2. **`crates/mika-cli/src/tui/commands/mod.rs:96`** — Update `args_hint` to `[<name|alias>]`
3. **`crates/mika-cli/src/commands/model.rs`** — Fix "Already using" check in `switch_model()` to handle prefixed aliases vs display model format
4. **`crates/mika-cli/src/tui/commands/handlers.rs`** — Update tests for new alias format
5. **`crates/mika-cli/src/tui/commands/completers.rs`** — Update test assertions

### "Already using" check in `switch_model()`

Currently at line 103-106:
```rust
let current_display = format_display_model(current_provider, &previous_model);
if full_id == current_display {
    bail!("Already using {display}.");
}
```

After normalization, `full_id` for `sonnet` becomes `anthropic/claude-sonnet-4-6`. On Anthropic, `current_display` is `claude-sonnet-4-6` (no prefix per `format_display_model`). These won't match. Fix: compare the resolved `(target_provider, model_name)` against `(current_provider, previous_model)` instead of comparing display strings.

### `resolve_model_alias()` in CLI path

At `cli.rs:686`, this function returns `full_id` directly. After normalization, `resolve_model_alias("sonnet")` returns `"anthropic/claude-sonnet-4-6"`. The CLI `--model` override handler in `chat.rs` must parse this with `parse_provider_model()` to correctly handle the provider prefix. Verify this path works.

## Acceptance Criteria

- [x] `MODEL_ALIASES` entries all include provider prefix (`anthropic/`, `openai/`, etc.)
- [x] `/model` args_hint updated to `[<name|alias>]`
- [x] `/model sonnet` on OpenRouter sets model to `anthropic/claude-sonnet-4-6` (valid OpenRouter model)
- [x] `/model sonnet` on Anthropic sets model to `claude-sonnet-4-6` (strips `anthropic/` prefix correctly)
- [x] `/model gpt4o` on Anthropic still switches provider to OpenAI (unchanged behavior)
- [x] "Already using" detection works for prefixed aliases on all providers
- [x] `mika model sonnet` CLI path works correctly
- [x] `mika ask --model sonnet` with OpenRouter config works correctly
- [x] Tab completion shows all aliases (still valid since all are now cross-provider compatible)
- [x] Existing tests updated and passing
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Sources

- Previous fix: `docs/solutions/ui-bugs/tui-provider-model-config-state-divergence.md` (#442)
- Previous fix: `docs/solutions/ui-bugs/tui-provider-switch-worker-propagation.md` (#451)
- Previous fix: `docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md` (#342-344)
- Three-file update rule: `mod.rs`, `handlers.rs`, `completers.rs`
- Key functions: `parse_provider_model()` (model.rs:150), `model_names_contain_slash()` (llm/mod.rs:250)
