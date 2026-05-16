---
module: kg/entity_resolver
tags: [kg, entity-resolution, outcome-split, observability, phantom-subjects]
problem_type: design_gap
category: kg
created: 2026-05-16
ticket: mika#1154
---

# KG resolver: no_candidate_of_type outcome split

## Problem

The resolver's `no_match` outcome conflated two structurally distinct
failure modes:

1. **Phantom subjects** — the extractor produced a subject (e.g.,
   `agent:vincent`, `tool:tailwind`) with no corresponding domain-graph
   entity to resolve to. Stage 1 exact match returns `Ok(None)`.

2. **Genuine disambiguation failures** — a domain entity of the
   subject's type exists (Stage 1 returns `Ok(Some)` with low
   confidence), but Stage 2 LLM disambiguation rejects it.

The mika#1152 experiment confirmed that 100% of sampled `no_match`
outcomes were case (1): zero domain-entity matches existed. The resolver
wasn't failing to disambiguate; there was nothing to disambiguate against.

## Solution

Split `no_match` into two outcomes based on Stage 1's result:

| Outcome | Condition | Meaning |
|---------|-----------|---------|
| `no_candidate_of_type` | Stage 1 `Ok(None)` + Stage 2 `Ok(None)` | Phantom subject — extractor error |
| `no_match` | Stage 1 `Ok(Some)` low-confidence + Stage 2 `Ok(None)` | Disambiguation failure — genuine ambiguity |

Additionally, the `candidates.is_empty()` short-circuit (no domain
entities of the subject's type exist at all) now maps to
`NoCandidateOfType` without issuing an LLM call.

## Key design decisions

1. **Stage 1's result is the signal, not a new query.** The plan's F3
   finding: `kg_entities`'s CHECK constraint (`entity_key = type || ':' || name`)
   makes Stage 1's case-insensitive `entity_key` lookup equivalent to a
   joint `(type, name)` match. No new index or query was needed — just
   tracking the existing result.

2. **Empty-candidates pre-check before `disambiguate_with_llm`.** The
   `candidates.is_empty()` logic was moved from inside the LLM function
   to the caller, yielding `NoCandidateOfType` with `used_llm=false`.
   This correctly avoids debiting the LLM budget for subjects that can
   never resolve.

3. **Domain rebuild invalidation covers both outcomes.** When new
   entities of a type are added, both `no_match` and
   `no_candidate_of_type` resolution log rows are deleted so those
   subjects get retried against the expanded domain graph. Without this,
   `no_candidate_of_type` subjects would be permanently stuck.

4. **Schema migration required.** The `kg_resolutions_log.outcome`
   column has an enumerating CHECK constraint. Adding a new value
   requires the SQLite table-rebuild pattern (RENAME → CREATE with wider
   CHECK → INSERT SELECT → DROP → recreate index). Migration: v34→v35.

## Pattern: sole-writer contracts as architectural ground

The decision to reject Shape 2 (widen domain roster via observation) was
grounded entirely on `domain_builder.rs`'s sole-writer contract
(doc-comment lines 12–21). This contract — naming the five entity-key
namespaces as exclusively owned by one module — was the only structural
argument. Pattern: doc-comments that name write ownership are
architectural decisions, not just documentation.

## Files changed

- `crates/mika-agent/src/kg/entity_resolver.rs` — outcome enum, stats,
  Stage 1 tracking, empty-candidates pre-check
- `crates/mika-agent/src/kg/domain_builder.rs` — invalidation query
  widened to include `no_candidate_of_type`
- `crates/mika-agent/src/db.rs` — schema v34→v35 migration
