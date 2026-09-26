---
module: kg/entity_resolver
tags: [kg, resolver, entity-resolution, db-fallback, candidate-list]
problem_type: logic-error
category: kg
issue: 874
milestone: 19
---

# KG Resolver: candidate-list check rejects valid LLM matches as no_match

## Problem

The entity resolver's Stage-2 LLM disambiguation validates the LLM-returned
`matched_key` against the in-prompt candidate list (max 50 entries, alphabetically
ordered). When the correct domain entity exists in `kg_entities` but falls outside
the 50-candidate window, the LLM may still emit the correct key from prompt context
(the subject's `entity_key` is always in the prompt), but the post-LLM validation
rejects it because it wasn't in the truncated candidate slice.

This produced hundreds of false `no_match` outcomes per batch, accounting for the
dominant share of mika-arch's 28,997-subject pending backlog.

## Root Cause

`disambiguate_with_llm` at `entity_resolver.rs:664-666` performed a single
`candidates.iter().find()` check. If the matched_key wasn't in the in-prompt slice,
the code fell through to a blanket `no_match` warning with no DB verification.

Two mechanically distinct causes produce the `matched_key == entity_key` log shape:
- **Cause A (this fix):** Valid match outside the 50-candidate window.
- **Cause B (sibling #875):** Stage-1 exact-match path broken upstream, flooding
  Stage-2 with entities whose correct match would have been caught at Stage-1.

## Solution

Added a 4-path validation taxonomy after the in-prompt candidate check fails:

| Path | Condition | Outcome | Log Event |
|------|-----------|---------|-----------|
| 1 | matched_key in in-prompt candidates | `matched_llm` | (silent) |
| 2 | matched_key in `kg_entities` same type (DB fallback) | `matched_llm_db_fallback` | INFO `resolution_matched_key_db_fallback_hit` |
| 3 | matched_key in `kg_entities` different type | `no_match` | WARN `resolution_matched_key_cross_type_rejected` |
| 4 | matched_key not in `kg_entities` at all | `no_match` | WARN `resolution_matched_key_not_in_candidates` (extended) |

### Key implementation details

- `try_domain_entity_by_key(entity_type, matched_key)`: Type-bounded DB lookup
  using range scan (same pattern as `get_domain_candidates`). Cross-type matches
  are rejected at the SQL level, not post-fetch filtering.
- `try_domain_entity_any_type(matched_key)`: Diagnostic-only query for Path 3
  cross-type detection.
- New `matched_llm_db_fallback` outcome in `kg_resolutions_log` CHECK constraint
  (schema v29->v30) so operator SQL (Signal C) can distinguish DB-fallback
  acceptance from in-prompt acceptance.
- `ResolutionStats` gains `matched_llm_db_fallback` counter; audit event JSON
  includes it.

### Schema migration

v29->v30: Table rebuild of `kg_resolutions_log` (RENAME -> CREATE -> INSERT INTO
SELECT -> DROP -> recreate index). Mirrors the v26->v27 shape. Single Immediate
transaction with PRAGMA foreign_keys OFF/ON.

## Tripwire

If `matched_llm_db_fallback` volume approaches or exceeds `matched_llm` across 3+
consecutive batches, the in-prompt 50-cap (`MAX_DISAMBIGUATION_CANDIDATES`) is too
tight and should be revisited. The new outcome makes this signal observable.

## Files Changed

- `crates/mika-agent/src/kg/entity_resolver.rs` — 4-path taxonomy, DB helpers,
  outcome const, stats
- `crates/mika-agent/src/db.rs` — v29->v30 migration, baseline CHECK update
- `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` — schema pin bump
- `crates/mika-agent/tests/schema_v27_convergence.rs` — version assertion update
