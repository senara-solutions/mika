---
status: complete
priority: p2
issue_id: "342"
tags: [code-review, performance, multimodal-tool-results]
dependencies: []
---

# Add Prefix Check Before JSON Parse in try_parse_envelope

## Problem Statement

`try_parse_envelope()` in `crates/mika-agent/src/skills/executor.rs` calls `serde_json::from_str()` on every exec handler output, even when the output is clearly not JSON (e.g., plain text, multi-line shell output). While the parse failure is fast for non-JSON, a simple prefix check could avoid the allocation and parsing overhead entirely.

## Findings

- **Source:** performance-oracle review agent
- **Severity:** P2 — minor optimization, but called on every exec handler invocation
- **Location:** `crates/mika-agent/src/skills/executor.rs` — `try_parse_envelope()` function
- **Evidence:** Every exec handler output is passed through `serde_json::from_str::<MikaEnvelope>()` regardless of content

## Proposed Solutions

### Solution A: Add prefix check for `{"__mika_v1"` (Recommended)

Before attempting JSON parse, check if the trimmed output starts with `{"__mika_v1"`. This is a fast string prefix check that avoids the JSON parser entirely for non-envelope output.

```rust
fn try_parse_envelope(output: &str) -> Option<MikaOutput> {
    let trimmed = output.trim();
    if !trimmed.starts_with(r#"{"__mika_v1""#) {
        return None;
    }
    serde_json::from_str::<MikaEnvelope>(trimmed)
        .ok()
        .map(|e| e.__mika_v1)
}
```

- **Pros:** Fast, simple, eliminates unnecessary JSON parsing for 99%+ of exec outputs
- **Cons:** Marginally less flexible (envelope must start with the sentinel key)
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Solution A

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/executor.rs`

## Acceptance Criteria

- [ ] Non-JSON exec output is not passed to serde_json
- [ ] Valid envelopes still parse correctly
- [ ] Existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Identified by performance-oracle agent |

## Resources

- PR branch: `feat/multimodal-tool-results`
