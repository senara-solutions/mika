---
status: complete
priority: p1
issue_id: 251
tags: [code-review, security, architecture]
dependencies: []
---

# Critic Auto-Approval on Parse Failure

## Problem Statement

When the critic agent's JSON response is unparseable, the multi-agent engine auto-approves the work (`engine.rs` line 417). Additionally, a missing `approved` field in the critic's response defaults to `true` (line 410). This makes the critic review step a no-op unless the critic explicitly returns `{"approved": false}`. A confused, malfunctioning, or prompt-injected critic silently approves everything.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 404-420
- **Severity:** P1 (Critical)
- **PR:** [#13](https://github.com/senara-solutions/mika/pull/13)

Two separate default-to-approve paths exist:

1. **Missing `approved` field (line 410):** The JSON parsing uses `unwrap_or(true)` when the `approved` field is absent. This means any JSON response that lacks the `approved` key (e.g., `{"feedback": "looks good"}`) is treated as approved.

2. **Unparseable JSON (line 417):** If the critic returns free-form text, malformed JSON, or anything that fails to parse, the catch-all path returns `Ok((true, ...))`, auto-approving the work.

Combined, these mean the critic can only reject work by returning well-formed JSON with an explicit `"approved": false`. Every other response — including errors, hallucinations, and prompt injection attempts — results in approval.

## Proposed Solutions

1. Change `unwrap_or(true)` to `unwrap_or(false)` for the `approved` field:

```rust
let approved = json["approved"].as_bool().unwrap_or(false);
```

2. Change auto-approve on parse failure to auto-reject:

```rust
// Instead of Ok((true, format!("...")))
Ok((false, format!("Critic response was not parseable JSON: {response}")))
```

## Technical Details

- The critic agent is expected to return JSON with `{"approved": bool, "feedback": "..."}` structure
- With fail-closed semantics, a malfunctioning critic causes re-review rather than silent approval
- This may increase retry loops if the critic frequently produces malformed output; consider adding a max-retries counter for critic reviews to prevent infinite loops
- The fail-closed approach follows the principle of least privilege: ambiguity should not grant access

## Acceptance Criteria

- [ ] Unparseable critic response results in rejection (not approval)
- [ ] Missing `approved` field in critic JSON results in rejection (not approval)
- [ ] Unit test: critic returns invalid JSON, verify task is rejected
- [ ] Unit test: critic returns JSON without `approved` field, verify task is rejected
- [ ] Unit test: critic returns `{"approved": true}`, verify task is approved
- [ ] Unit test: critic returns `{"approved": false}`, verify task is rejected
- [ ] Consider adding max-retries for critic review loops to prevent infinite rejection cycles

## Work Log

- 2026-02-25: Finding identified during code review of PR #13

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Fail-closed vs fail-open security design: https://en.wikipedia.org/wiki/Fail-safe
