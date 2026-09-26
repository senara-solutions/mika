---
status: complete
priority: p2
issue_id: "636"
tags: [code-review, security, rewind]
dependencies: []
---

# REWIND_MARKER_PREFIX LIKE pattern could match natural language

## Problem Statement

`delete_rewind_markers` uses `LIKE '[Context notice: A rewind%'` to find markers. If an assistant or user message happened to start with this exact prefix, it would be incorrectly deleted. The prefix is distinctive but not structurally guaranteed unique.

## Findings

- **Source:** Security review agent (MEDIUM severity)
- **Location:** `crates/mika-agent/src/rewind.rs` (`REWIND_MARKER_PREFIX`), `crates/mika-agent/src/db.rs` (`delete_rewind_markers`)
- The LIKE query also filters on `role = 'system'`, which significantly reduces collision risk (only system-injected messages)
- Combined `role = 'system' AND content LIKE prefix` makes false matches extremely unlikely in practice

## Proposed Solutions

### Option A: Add a structured sentinel prefix
Use something like `[__mika_rewind_v1__: A rewind operation...]` — structurally unique, not natural language.
- **Effort:** Small
- **Risk:** Low

### Option B: Accept current approach
The `role = 'system'` filter combined with the specific prefix makes collisions practically impossible.
- **Effort:** None
- **Risk:** Very low

## Acceptance Criteria

- [ ] Either: Marker prefix uses a structural sentinel, OR: documented as acceptable given role filter
