---
status: complete
priority: p3
issue_id: "588"
tags: [code-review, architecture]
dependencies: []
---

# Eliminate Response Type Ceremony (~180 Lines)

## Problem Statement
7 response structs + 7 From impls copy DB struct fields 1:1 with no transformation. Adding `#[derive(Serialize)]` to DB types and `#[serde(skip)]` on `home_dir` would eliminate ~180 lines.

## Findings
- **Source:** Code Simplicity Reviewer
- **Location:** `crates/mika-agent/src/server/dashboard.rs` (response types throughout)
- Only `MessageResponse` has a transformation (`strip_base64_images`), which can be done inline

## Proposed Solutions
Derive Serialize on DB structs, handle the one transformation inline.

## Acceptance Criteria
- [ ] Response wrapper types eliminated where they add no value
- [ ] `home_dir` not leaked in API responses

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Simplicity Reviewer found ~180 lines of ceremony |

## Resources
- PR #89
