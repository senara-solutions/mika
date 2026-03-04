---
status: complete
priority: p3
issue_id: "437"
tags: [code-review, architecture, quality]
dependencies: []
---

# Replace Sentinel Error String with Typed Return

## Problem Statement

`parse_task_assignments()` uses `bail!("__conversational__:{reply}")` as control flow, caught by `execute_inner()` via `msg.strip_prefix("__conversational__:")`. This conflates errors with valid result variants and relies on magic string matching.

## Findings

- 5 of 7 review agents flagged this pattern independently
- Currently well-tested (3 test cases) but brittle
- Any refactoring of error message formatting could silently break detection

## Proposed Solutions

### Option A: Return typed enum (Recommended)

```rust
enum DecomposeResult {
    Tasks(Vec<TaskAssignment>),
    Conversational(String),
}
```

Have `decompose()` return `Result<DecomposeResult>`.

- **Pros:** Type-safe, self-documenting, compile-time checks
- **Cons:** Slightly more code
- **Effort:** Medium
- **Risk:** Low

## Technical Details

- **File:** `crates/mika-agent/src/teams/engine.rs`, lines ~247-252, ~748-754
- **Components:** Team orchestration decompose/execute flow

## Acceptance Criteria

- [ ] `__conversational__:` sentinel removed
- [ ] Typed enum replaces error-based control flow
- [ ] Existing tests updated
