---
title: "chore(docs): audit and correct stale #265 references across mika-agent"
type: chore
status: active
date: 2026-05-12
---

# chore(docs): audit and correct stale #265 references across mika-agent

## Overview

Replace incorrect `#265` citations with `#463` across mika-agent code comments and CLAUDE.md. mika#265 is about `mika ask` vs TUI execution; mika#463 is the actual precedent for `MatchReason::Keyword`-only filtering. Multiple code sites incorrectly cite #265 for the Keyword-only filter design decision.

## Problem Frame

Discovered during mika#1011 investigation: the PR for #1011 corrected one specific `agent.rs` citation, but 10 other sites still reference #265 when they mean #463. This creates misleading provenance for anyone tracing the design history of MatchReason-based constraint scoping.

## Requirements Trace

- R1. Every `#265` reference in mika-agent source and CLAUDE.md is verified against the actual issue content
- R2. References about MatchReason/Keyword-only filtering are corrected to cite #463
- R3. The one legitimate #265 reference (db.rs — `mika ask` pending callback detection) is preserved

## Scope Boundaries

- Comments and documentation only — no logic changes
- Scoped to `crates/mika-agent/` (source + CLAUDE.md)

## Context & Research

### Relevant Code and Patterns

- mika#265: "`mika ask` announces implementation but doesn't execute" — about headless vs TUI execution mode
- mika#463: "`resolve_skill_llm_override` should filter by `MatchReason::Keyword` only" — the actual precedent for Keyword-only scoping of skill constraints
- mika#1011: origin of this audit (corrected one site; this ticket covers the remaining 10)

## Key Technical Decisions

- **Replace #265 → #463 for MatchReason sites**: All 10 sites citing #265 in the context of `MatchReason`, `required_tools`, or `always_on` constraint scoping should cite #463 instead
- **Keep db.rs:4368 as-is**: This reference is legitimately about mika#265 — the function `get_pending_callbacks_for_session` detects callbacks spawned during `mika ask` headless execution, which is exactly the problem #265 describes

## Implementation Units

- [x] **Unit 1: Correct source code comments**

**Goal:** Replace 8 incorrect `#265` references in Rust source files with `#463`

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/matcher.rs` (lines 8, 201)
- Modify: `crates/mika-agent/src/skills/index.rs` (line 836)
- Modify: `crates/mika-agent/src/agent.rs` (lines 4023, 4042, 6575, 6623, 6769)
- Keep unchanged: `crates/mika-agent/src/db.rs` (line 4368 — legitimate #265 reference)

**Approach:**
- `matcher.rs:8`: doc comment on `MatchReason` enum — change "See #265" → "See #463"
- `matcher.rs:201`: test section header — change "(#265)" → "(#463)"
- `index.rs:836`: always_on warning comment — change "(#265)" → "(#463)"
- `agent.rs:4023`: `collect_required_tools` doc comment — change "See #265, #270" → "See #463, #270"
- `agent.rs:4042`: pre-fetch scoping comment — change "per #265" → "per #463"
- `agent.rs:6575`: test section header — change "(#270, #265)" → "(#270, #463)"
- `agent.rs:6623`: test comment — change "(#265)" → "(#463)"
- `agent.rs:6769`: test comment — change "(#265)" → "(#463)"

**Test expectation:** none — comments-only changes, no behavioral impact

**Verification:**
- `grep -rn '#265' crates/mika-agent/src/` returns only the legitimate `db.rs:4368` reference
- `grep -rn '#463' crates/mika-agent/src/` returns 8 new references plus any pre-existing ones

- [x] **Unit 2: Correct CLAUDE.md references**

**Goal:** Replace 2 incorrect `#265` references in `crates/mika-agent/CLAUDE.md` with `#463`

**Requirements:** R1, R2

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` (lines 27, 190)

**Approach:**
- Line 27: required-tools gate paragraph — change "(#265)" → "(#463)"
- Line 190: Match-reason conditioning heading — change "(#265)" → "(#463)"

**Test expectation:** none — documentation-only changes

**Verification:**
- `grep -n '#265' crates/mika-agent/CLAUDE.md` returns zero results
- `grep -n '#463' crates/mika-agent/CLAUDE.md` returns at least 2 results

## System-Wide Impact

- **Interaction graph:** None — no behavioral changes
- **Error propagation:** N/A
- **State lifecycle risks:** None
- **API surface parity:** N/A
- **Unchanged invariants:** All runtime behavior unchanged; only provenance comments corrected

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Line numbers may have drifted since audit | Grep for the exact comment text, not line numbers |

## Sources & References

- Related issues: mika#265, mika#463, mika#1011, mika#1012
