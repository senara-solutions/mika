---
title: "fix: Strip <think> tag from reasoning-model CoT output"
type: fix
origin: "GitHub issue #750"
date: 2026-06-12
depth: Lightweight
---

# fix: Strip `<think>` tag from reasoning-model CoT output

## Summary

Add `"think"` to the `INTERNAL_TAG_NAMES` constant in `crates/mika-common/src/llm/mod.rs` so that `strip_internal_tags()` removes reasoning-model chain-of-thought blocks (`<think>…</think>`) before responses reach users. This prevents CoT content (often in non-English languages) and raw `</think>` tags from leaking into Telegram, TUI, and dashboard output.

## Problem Frame

Reasoning models (kimi-k2.5, DeepSeek-R1, Qwen variants) emit `<think>…</think>` blocks containing their chain-of-thought before the final answer. The existing `strip_internal_tags()` function removes engine-injected XML tags but does not include `think` in its tag list. Result: CoT text and closing tags leak into user-facing responses. Observed in production on 2026-04-23 when mika-dev (on kimi-k2.5 via OpenRouter) sent Thai-language reasoning followed by `</think>` through the Telegram gateway.

## Requirements

- R1. `<think>…</think>` blocks stripped from LLM responses before display and persistence
- R2. Malformed variants (bare `</think>`, whitespace like `< /think>`) handled by existing regex infrastructure
- R3. No regression in existing `strip_internal_tags` behavior
- R4. Unit test coverage for well-formed, bare closing tag, non-English CoT, and multiple block cases

## Key Technical Decisions

- **Array extension, not new regex.** The existing `build_tag_regex()` and `INTERNAL_TAG_RES` lazy compilation handle all tag variants automatically. Adding `"think"` to the array is sufficient — no new regex logic needed.
- **No CoT preservation for observability.** Out of scope per issue #750. A separate ticket can route CoT to logs while stripping from user output.

## Implementation Units

### U1. Add `think` to tag list and tests

**Goal:** Strip `<think>` blocks from LLM responses; verify with unit tests.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- `crates/mika-common/src/llm/mod.rs` (modify — add array entry + tests)

**Approach:** Add `"think"` to `INTERNAL_TAG_NAMES` at `crates/mika-common/src/llm/mod.rs:31`. The existing `build_tag_regex()` handles dotall matching and malformed closing-tag tolerance (whitespace, bare form) — no changes needed there. Add unit tests in the existing `strip_internal_tags` test block.

**Patterns to follow:** Existing tests at the same location (`test_strip_context_tag`, `test_strip_malformed_closing_bare_tag`, etc.) — same assertion style and naming convention.

**Test scenarios:**
1. Well-formed `<think>CoT in English</think>actual answer` → output `"actual answer"`
2. Non-English CoT body (Thai/Chinese) inside `<think>` — stripped cleanly, only English answer remains
3. Bare closing-tag variant: text followed by orphan `</think>` without opening tag — the existing regex matches `<think>…think>` form, so a bare `</think>` following an opening `<think>` is handled. An orphan `</think>` with no opening tag is left alone (consistent with existing behavior for other tags)
4. Multiple `<think>` blocks in one response — each stripped independently
5. `<think>` with attributes (e.g., `<think type="cot">`) — stripped (regex uses `\b[^>]*>`)
6. Existing tag tests still pass (no regression)

**Verification:** `cargo test -p mika-common` passes with all new and existing tests green.
