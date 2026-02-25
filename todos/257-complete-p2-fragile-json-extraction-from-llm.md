---
status: complete
priority: p2
issue_id: 257
tags: [code-review, architecture, quality]
dependencies: []
---

# Fragile JSON Extraction from LLM Responses

## Problem Statement

`extract_json_array()` and `extract_json_object()` use naive bracket matching (find first `[`, last `]` or first `{`, last `}`). This fails when LLM responses contain bracket characters in surrounding prose text. For example, "Here are [some notes]: [actual JSON]" would extract from the first `[` to the last `]`, capturing invalid content.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 422-441
- `extract_json_array()` finds the first `[` and last `]` in the text and attempts to parse the substring
- `extract_json_object()` finds the first `{` and last `}` in the text and attempts to parse the substring
- LLM responses frequently include natural language before/after JSON, which may contain bracket characters
- The two functions are nearly identical, differing only in the delimiter characters
- No fallback or retry logic if the first extraction attempt fails

## Proposed Solutions

1. **More specific patterns:** Search for `[{` (array of objects) and `{"` (object with string key) as start markers, which are far less likely to appear in prose
2. **Code fence extraction:** Instruct Claude to wrap output in markdown code fences (```json ... ```) and extract from those first, falling back to bracket matching
3. **Merge functions:** Combine `extract_json_array()` and `extract_json_object()` into a single parameterized `extract_json<T>(text) -> Result<T>` that uses `serde_json::from_str` with type inference
4. **Progressive parsing:** Try parsing increasingly large substrings, or use a proper JSON-aware scanner

Recommended approach: Combine options 1 and 3 -- use smarter start markers and a single generic function.

```rust
fn extract_json<T: DeserializeOwned>(text: &str, open: char, close: char) -> Result<T> {
    // Try code fence extraction first
    if let Some(json_str) = extract_from_code_fence(text) {
        if let Ok(parsed) = serde_json::from_str(json_str) {
            return Ok(parsed);
        }
    }
    // Fall back to bracket matching with smarter start markers
    // ...
}
```

## Technical Details

- The LLM is Claude, which generally follows formatting instructions well -- code fence approach should be reliable
- The JSON being extracted is structured (task assignments, plans, summaries) so the start markers are predictable
- A generic function with `DeserializeOwned` bound would reduce code duplication and improve type safety
- Consider adding test cases with adversarial LLM output containing brackets in prose

## Acceptance Criteria

- [ ] JSON extraction handles text with bracket characters in surrounding prose
- [ ] Tests cover cases like "Here are [notes]: [actual JSON]" and "Result {summary}: {actual JSON}"
- [ ] The two extraction functions are merged into one parameterized function
- [ ] Existing extraction behavior is preserved for well-formed inputs
- [ ] All existing tests pass

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
