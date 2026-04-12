---
status: pending
priority: p3
issue_id: 760
tags: [code-review, architecture, tech-debt]
dependencies: []
---

# `save_message_with_metadata` has too many positional parameters

## Problem Statement

`Database::save_message_with_metadata()` now has 8 positional parameters (self, agent_id, session_id, role, content, metadata, trace_id, internal) after #494 added the `internal: bool` flag. A `#[allow(clippy::too_many_arguments)]` suppression was added.

## Findings

- Several `Option<&str>` params make callsites hard to read
- The clippy suppression is pragmatic but signals the API is at its limit
- Any future parameter additions will worsen readability

## Proposed Solutions

### Option A: SaveMessageParams struct
Replace positional params with a builder or params struct.

- **Pros:** Cleaner callsites, self-documenting field names, extensible
- **Cons:** More code, all callers need updating
- **Effort:** Medium
- **Risk:** Low

### Option B: Leave as-is
The function is an internal DB helper, not a public API. The suppression is fine.

- **Pros:** No churn
- **Cons:** Readability won't improve
- **Effort:** None
- **Risk:** None

## Acceptance Criteria

- [ ] Decide on approach
- [ ] If Option A: refactor and remove clippy suppression
