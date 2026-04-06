---
title: Malformed closing tags from non-Anthropic models leak into TUI
category: ui-bugs
date: 2026-04-06
tags: [agent-loop, llm-response, tag-stripping, regex, non-anthropic, openrouter, deepseek, qwen]
issue: "#453"
related:
  - docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md
  - docs/solutions/runtime-errors/xml-tool-calls-not-executed.md
---

# Malformed closing tags from non-Anthropic models leak into TUI

## Problem

Non-Anthropic models (DeepSeek V3.2, Qwen via OpenRouter) sometimes echo internal XML context tags back in their responses with corrupted closing tags. The closing tag `</context>` gets mangled to `context>` (missing `</`), so `strip_internal_tags()` doesn't match and raw metadata leaks into the TUI display.

**Symptom:**
```
Active sprint items (self-dev) completed.
<context type="tool_history" trust="metadata">
list_work_items({"source":"self_dev","status":"in_progress"}) → No work items found...
check_work_item({"task_id":"bfaf7b8a..."}) → Work item bfaf7b8a...
context>
```

## Root Cause

`strip_internal_tags()` in `mika-common::llm` used per-tag regexes with pattern `(?s)<{tag}\b[^>]*>.*?</{tag}>` which requires a well-formed `</tag>` closing. LLM tokenization can split `</` across token boundaries, producing:

- `context>` — bare tag name without `</`
- `< /context>` — space after `<`
- `</ context>` — space before tag name

These are LLM tokenization artifacts, not XML parsing errors. Non-Anthropic providers are particularly susceptible because they handle system prompts and injected context differently.

## Solution

Widened `build_tag_regex()` closing pattern to use alternation:

```rust
fn build_tag_regex(tag: &str) -> Regex {
    Regex::new(&format!(
        r"(?s)<{tag}\b[^>]*>.*?(?:<\s*/\s*{tag}\s*>|{tag}\s*>)"
    ))
    .expect("tag regex must compile")
}
```

Two closing branches:
- `<\s*/\s*{tag}\s*>` — standard closing with optional whitespace (`</tag>`, `< /tag>`, `</ tag>`)
- `{tag}\s*>` — bare tag name without `</` prefix (`tag>`, `tag >`)

**Design decisions:**
- No `(?i)` case-insensitivity — increases false-positive risk for `context` (common English word)
- No unclosed-tag stripping — completely absent closing tags are left alone (existing behavior)
- Fast path unchanged — `!text.contains('<')` still correct because opening tag always requires `<`
- Bare `{tag}>` false-positive risk accepted — requires both opening `<tag...>` and bare `tag>` in same text; lazy `.*?` limits match to shortest span

## Prevention

- Non-Anthropic models are unreliable with XML-like content in responses. Always design tag-stripping regexes to tolerate malformed variants.
- When adding new internal XML tags, consider what malformed closings might look like and test with the alternation pattern.
- Use `MIKA_LOG_LLM_BODIES=true` to capture raw provider responses when debugging tag-stripping issues.
- The `extract_xml_tool_calls()` function in `openai.rs` handles a related but distinct problem (XML tool calls instead of structured tool_calls) — same root cause of non-Anthropic providers emitting XML-like text.
