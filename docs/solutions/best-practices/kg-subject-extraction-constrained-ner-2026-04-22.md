---
title: "Subject graph extraction uses constrained NER with structural validation"
date: 2026-04-22
category: best-practices
module: kg
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding LLM-based extraction to a knowledge graph pipeline
  - Building NER systems that produce typed entities and relationships
  - Designing extraction output schemas for graph ingestion
tags:
  - knowledge-graph
  - subject-extraction
  - llm-extraction
  - ner
  - constrained-types
  - validation
---

# Subject graph extraction uses constrained NER with structural validation

## Context

Mika's Knowledge Graph (#690) needs to extract named entities and fact triples from solution docs using LLM-based NER. The extracted entities must conform to an approved type system (`skill`, `tool`, `agent`, `problem_type`, `solution_path`, `failure_mode`, `pattern`) and relationships must follow directional constraints (e.g., `SOLVED_BY` only goes from `problem_type` to `solution_path`).

Relying solely on prompt instructions to enforce these constraints is fragile — LLMs hallucinate types, ignore directional rules, and produce malformed JSON. The `feedback_prompt_enforcement_fragile.md` solution documents this pattern.

## Guidance

Use a two-layer validation strategy: **prompt instructs, code enforces**.

1. **Prompt layer**: Tell the LLM the approved types, relationship constraints, and output schema. This guides output quality but is not trusted for correctness.

2. **Structural validation layer**: After parsing the JSON response, validate every entity type against `APPROVED_ENTITY_TYPES`, every relationship type against `APPROVED_RELATIONSHIP_TYPES` with from/to type constraints, and reject entities with names containing `:` (which would break the `entity_key = type || ':' || name` convention).

3. **Partial acceptance**: When some entities pass and others fail validation, keep the valid subset rather than rejecting the entire extraction. This maximizes extraction yield from a single LLM call. Only reject entirely when ALL entities are invalid.

4. **UPSERT semantics**: Use `INSERT ... ON CONFLICT DO UPDATE` with `MAX(excluded.confidence, existing.confidence)` to preserve the highest confidence seen across extractions. This makes extraction idempotent — re-extracting the same doc produces the same result.

5. **Provenance tracking**: Every entity and relationship links back to source chunks via join tables (`kg_chunk_subjects`, `kg_chunk_subject_relationships`). This enables the three-phase re-extraction reconciliation (D5) — capture previous provenance, reingest chunks, re-extract with scoped orphan sweep.

6. **Tracking table for pending detection**: A `kg_extractions` table records which docs have been extracted. The pending-doc query joins `kg_chunks` against `kg_extractions` to find docs needing extraction. Zero-entity docs get a tracking row with `entities_extracted = 0` — without this, they would re-extract on every startup.

## Why This Matters

Prompt-only enforcement leads to invalid graph edges that downstream consumers (query tool, self-knowledge) treat as truth. A `failure_mode:fabrication -[SOLVED_BY]-> skill:self_dev` edge with wrong types would mislead the agent's self-knowledge answers. Structural validation catches these at extraction time, before they enter the graph.

The UPSERT + provenance pattern makes the extractor idempotent and crash-safe — if the server crashes mid-extraction, no tracking row is written and the doc remains pending for next startup. No manual cleanup required.

## When to Apply

- Building any LLM-based extraction pipeline that feeds a typed graph or database
- When extraction output must conform to a schema with enum constraints
- When the same document may be re-extracted (content changes, model upgrades)
- When extraction needs to be crash-safe and resumable

## Examples

Approved relationship constraints with from/to type enforcement:

```rust
pub const APPROVED_RELATIONSHIP_TYPES: &[RelationshipConstraint] = &[
    RelationshipConstraint {
        rel_type: "SOLVED_BY",
        from_types: &["problem_type"],
        to_types: &["solution_path"],
    },
    // ... other constrained types
];
```

Validation filters invalid items rather than rejecting the whole response:

```rust
// If some entities pass and some fail, return the valid subset
if valid_entities.is_empty() && !output.entities.is_empty() {
    return Err(ValidationError { ... }); // All rejected = error
}
// Otherwise return partial results with warnings
Ok(ExtractionOutput {
    entities: valid_entities,
    relationships: valid_relationships,
})
```

## Related

- `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md` — lexical layer composed write pattern
- `docs/solutions/best-practices/kg-domain-graph-startup-projection-2026-04-22.md` — domain layer deterministic projection
- `docs/architecture/kg-implementation-conventions.md` — cross-cutting KG conventions (C2 LLM policy, C3 observability)
- `docs/plans/2026-04-21-006-feat-subject-graph-extraction-plan.md` — full implementation plan
