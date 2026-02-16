---
status: complete
priority: p3
issue_id: 261
tags: [code-review, quality, simplification]
dependencies: []
---

# Merge Duplicate Engine Methods

## Problem Statement

Two simplification opportunities exist in the engine module:

1. `decompose()` and `decompose_with_feedback()` are nearly identical methods that differ only in whether previous feedback is passed.
2. `extract_json_array()` and `extract_json_object()` contain identical logic with different bracket characters.

## Findings

- `decompose()` and `decompose_with_feedback()` share the same core logic for task decomposition via the Claude API. The only difference is that `decompose_with_feedback()` includes previous feedback in the prompt. This duplication means any change to decomposition logic must be applied in two places.
- `extract_json_array()` and `extract_json_object()` both scan text for matching brackets and extract the enclosed JSON. The only difference is the open/close bracket characters (`[`/`]` vs `{`/`}`).

## Proposed Solutions

1. **Merge decompose methods:** Combine into a single `decompose(feedback: Option<&str>)` method. When `feedback` is `Some`, include it in the prompt; when `None`, omit it. Callers that previously used `decompose()` pass `None`, and callers that used `decompose_with_feedback()` pass `Some(feedback)`.

2. **Merge JSON extractors:** Create a single `extract_json(text: &str, open: char, close: char)` function. `extract_json_array()` becomes `extract_json(text, '[', ']')` and `extract_json_object()` becomes `extract_json(text, '{', '}')`. Optionally keep the specific functions as thin wrappers for readability.

Estimated ~24 lines saved.

## Technical Details

**Files affected:**
- `crates/mika-agent/src/teams/engine.rs`

**Decompose merge:**
```rust
// Before: two methods
fn decompose(&self, task: &str) -> Result<Vec<SubTask>>
fn decompose_with_feedback(&self, task: &str, feedback: &str) -> Result<Vec<SubTask>>

// After: single method with optional feedback
fn decompose(&self, task: &str, feedback: Option<&str>) -> Result<Vec<SubTask>>
```

**JSON extractor merge:**
```rust
// Before: two functions
fn extract_json_array(text: &str) -> Option<&str>
fn extract_json_object(text: &str) -> Option<&str>

// After: single parameterized function
fn extract_json(text: &str, open: char, close: char) -> Option<&str>
```

## Acceptance Criteria

- [ ] Single `decompose` method with `feedback: Option<&str>` parameter
- [ ] Single `extract_json` function parameterized by bracket characters
- [ ] All existing callers updated
- [ ] All tests pass (`cargo test`)
- [ ] No change in behavior

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from code review of PR #13 |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
