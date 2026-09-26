---
status: complete
priority: p2
issue_id: 369
tags: [code-review, publishing, crates-io, metadata]
dependencies: []
---

# Inconsistent publishing metadata across crates

## Problem Statement

The four crates have inconsistent sets of publishing metadata fields. `mika-ai` has the most complete set while `mika-common` has the least. The workspace defines `homepage`, `keywords`, and `categories` but not all crates inherit them.

## Findings

| Field | mika-ai | mika-agent | mika-common | mika-gateway |
|---|---|---|---|---|
| `description` | yes | yes | yes | yes |
| `repository.workspace` | yes | yes | yes | yes |
| `homepage.workspace` | yes | **missing** | **missing** | **missing** |
| `keywords.workspace` | yes | yes | **missing** | yes |
| `categories.workspace` | yes | yes | **missing** | yes |
| `readme` | yes | yes | **missing** | yes |

`mika-common` is missing 4 optional fields. `mika-agent` and `mika-gateway` are each missing `homepage`.

Note: `categories = ["command-line-utilities"]` may not be appropriate for the `mika-common` library crate.

## Proposed Solutions

### Option 1: Add missing fields for full consistency
- Add `homepage.workspace = true` to mika-agent, mika-common, mika-gateway
- Add `keywords.workspace = true`, `categories.workspace = true`, `readme = "../../README.md"` to mika-common
- Use a different category for mika-common if "command-line-utilities" doesn't fit
- **Pros:** Full consistency; better crates.io presentation
- **Cons:** `categories` may not apply to library crate
- **Effort:** Small
- **Risk:** Low

### Option 2: Accept intentional variation
- `mika-common` is a library — fewer fields is reasonable
- `homepage` duplicates `repository` anyway — skip it
- **Pros:** No changes needed; less noise
- **Cons:** Looks like an oversight on crates.io
- **Effort:** None
- **Risk:** Low

## Recommended Action

(To be decided during triage)

## Technical Details

- **Affected files:** `crates/mika-common/Cargo.toml`, `crates/mika-agent/Cargo.toml`, `crates/mika-gateway/Cargo.toml`

## Acceptance Criteria

- [ ] Deliberate decision on which fields each crate should have
- [ ] All crates have consistent metadata (or documented reason for variance)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Architecture, pattern, and simplicity reviewers all flagged this |

## Resources

- Commit: 2eca502 "Prepare crates for publishing to crates.io"
- [crates.io manifest fields](https://doc.rust-lang.org/cargo/reference/manifest.html)
