---
status: pending
priority: p2
issue_id: 709
tags: [code-review, quality]
dependencies: []
---

# Duplicated extract_text_from_parts function

## Problem Statement
The `extract_text_from_parts` function exists identically in two files: `a2a_db.rs` (line 36) and `server/a2a.rs` (line 833). This violates DRY and risks divergence.

## Findings
- `crates/mika-agent/src/a2a_db.rs` lines 36-45
- `crates/mika-agent/src/server/a2a.rs` lines 833-842
- Both implementations are character-for-character identical

## Proposed Solutions
Make the `a2a_db.rs` version `pub(crate)` and import it in `server/a2a.rs`.

## Acceptance Criteria
- [ ] Single implementation of `extract_text_from_parts` used from both locations
