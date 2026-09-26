---
title: "fix: strip_internal_tags fails on malformed closing tags"
type: fix
status: completed
date: 2026-04-06
issue: "#453"
---

# fix: strip_internal_tags fails on malformed closing tags

## Overview

Non-Anthropic models (DeepSeek V3.2, Qwen via OpenRouter) sometimes echo internal XML context tags back in their responses with corrupted closing tags — e.g., `context>` instead of `</context>`. The current `strip_internal_tags()` regex requires well-formed `</tag>` closings, so malformed variants pass through and leak raw metadata into the TUI display.

## Problem Statement

`strip_internal_tags()` in `crates/mika-common/src/llm/mod.rs` uses per-tag regexes built by `build_tag_regex()`:

```rust
fn build_tag_regex(tag: &str) -> Regex {
    Regex::new(&format!(r"(?s)<{tag}\b[^>]*>.*?</{tag}>")).expect("tag regex must compile")
}
```

The pattern `(?s)<{tag}\b[^>]*>.*?</{tag}>` requires a well-formed closing tag `</tag>`. When models produce `context>` (missing `</`), `< /context>` (space after `<`), or `</ context>` (space before tag name), the regex fails to match.

**Observed in production:**
```
Active sprint items (self-dev) completed.
<context type="tool_history" trust="metadata">
list_work_items({"source":"self_dev","status":"in_progress"}) → No work items found...
check_work_item({"task_id":"bfaf7b8a..."}) → Work item bfaf7b8a...
context>
```

## Proposed Solution

Widen `build_tag_regex()` to accept three closing patterns via alternation:

1. **Well-formed:** `</tag>` (existing)
2. **Whitespace-tolerant:** `< /tag>`, `</ tag>`, `< / tag >` (LLM tokenization artifacts)
3. **Bare tag name:** `tag>` without `</` prefix (observed in #453)

Updated regex template:
```rust
fn build_tag_regex(tag: &str) -> Regex {
    Regex::new(&format!(
        r"(?s)<{tag}\b[^>]*>.*?(?:<\s*/\s*{tag}\s*>|{tag}\s*>)"
    )).expect("tag regex must compile")
}
```

### Pattern breakdown

- `<\s*/\s*{tag}\s*>` — standard closing with optional whitespace: `</tag>`, `< /tag>`, `</ tag>`, `< / tag >`
- `{tag}\s*>` — bare tag name closing without `<`: `tag>`, `tag >`

### Design decisions

1. **No case-insensitivity (`(?i)`)** — for `context`, case-insensitive matching increases false-positive risk (common English word). No evidence of case-changed closings.
2. **No unclosed-tag stripping** — completely absent closing tags remain unstripped (existing behavior, protected by `test_strip_partial_unclosed_tag_left_alone`). Greedy fallback would risk eating legitimate content.
3. **No opening-tag corruption handling** — LLMs faithfully echo long opening tags with attributes; corruption is rare. Would require fundamentally different matching.
4. **Fast path unchanged** — `!text.contains('<')` still works because the opening tag always requires `<`.
5. **Bare `{tag}>` false-positive risk accepted** — for `context>` to false-positive, text must contain both an opening `<context...>` tag AND a bare `context>` after it. The lazy `.*?` limits the match to the shortest span. All other 6 tag names are synthetic and cannot appear in natural prose.

## Acceptance Criteria

- [x] `strip_internal_tags()` removes `<context>` blocks with malformed closing `context>` (the reported case)
- [x] Handles whitespace variants: `< /context>`, `</ context>`, `< / context >`
- [x] Handles bare closing with trailing space: `context >` 
- [x] Existing 12 tests still pass — no regressions
- [x] New test: malformed closing tag `context>` (from issue)
- [x] New test: whitespace in closing `</ context>`
- [x] New test: nested `<task-health>` with malformed outer closing
- [x] Unclosed tags still left alone (regression guard)
- [x] TUI does not display raw `<context type="tool_history">` blocks

## Implementation

### File: `crates/mika-common/src/llm/mod.rs`

**Change 1 — Update `build_tag_regex()` (line 39-41):**

```rust
fn build_tag_regex(tag: &str) -> Regex {
    // Match opening tag with any attributes, then content (lazy), then closing tag.
    // Closing tag tolerates malformed variants from non-Anthropic models:
    //   - Well-formed: </tag>
    //   - Whitespace: < /tag>, </ tag>, < / tag >
    //   - Bare: tag> (missing </)
    Regex::new(&format!(
        r"(?s)<{tag}\b[^>]*>.*?(?:<\s*/\s*{tag}\s*>|{tag}\s*>)"
    ))
    .expect("tag regex must compile")
}
```

**Change 2 — Add test cases (after line 567):**

```rust
#[test]
fn test_strip_malformed_closing_bare_tag() {
    // #453: non-Anthropic models echo `context>` instead of `</context>`
    let input = r#"Hello.
<context type="tool_history" trust="metadata">
list_work_items({"source":"self_dev"}) → No work items found...
context>
Bye."#;
    assert_eq!(strip_internal_tags(input), "Hello.\n\nBye.");
}

#[test]
fn test_strip_malformed_closing_space_in_slash() {
    // LLM tokenization artifact: </ context> with space before tag name
    let input = r#"Result: <context type="summary">data here</ context> done."#;
    assert_eq!(strip_internal_tags(input), "Result:  done.");
}

#[test]
fn test_strip_malformed_closing_space_after_angle() {
    // LLM tokenization artifact: < /context> with space after <
    let input = r#"X <callback_result trust="untrusted">result< /callback_result> Y"#;
    assert_eq!(strip_internal_tags(input), "X  Y");
}

#[test]
fn test_strip_nested_with_malformed_outer_closing() {
    // Outer <task-health> has malformed closing, inner tags are well-formed
    let input = "<task-health>\n<active-work-items>\n- item\n</active-work-items>\ntask-health>";
    assert_eq!(strip_internal_tags(input), "");
}
```

## Sources

- Related issue: #453
- Related: #447 (XML tool call extraction — different format)
- Solution doc: `docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md`
- Solution doc: `docs/solutions/runtime-errors/xml-tool-calls-not-executed.md`
