---
status: complete
priority: p2
issue_id: "639"
tags: [code-review, security, rewind]
dependencies: []
---

# Reversal descriptions contain unsanitized user-edited audit content

## Problem Statement

Reversal descriptions are built from audit event data which contains user-edited content (memory facts, person names, preferences). This content is injected into a system message that the agent will process. A malicious or adversarial memory value could embed prompt injection in the marker.

## Findings

- **Source:** Learnings research agent
- **Location:** `crates/mika-agent/src/rewind.rs` — `build_reversal_previews()` and `build_rewind_marker()`
- Audit event `new_value`/`old_value` fields contain raw user-provided text
- The marker is saved as `role='system'` → mapped to `role='user'` for Claude API
- Similar to the existing pattern where callback results use `<callback_result trust="untrusted">` wrapper

## Proposed Solutions

### Option A: Wrap reversal descriptions in trust boundary tags
Use `<rewind_context trust="internal">` wrapper similar to callback result pattern.
- **Effort:** Small
- **Risk:** Low

### Option B: Truncate and sanitize descriptions
Limit description length and strip control characters / XML-like tags.
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [x] Reversal descriptions are wrapped in `<rewind_reversals trust="internal">` trust boundary tags
- [x] Individual descriptions truncated to 200 chars max via `truncate_content()`
