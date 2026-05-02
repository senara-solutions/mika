---
title: "KG domain graph concept namespace expansion for cross-repo and infrastructure coverage"
date: 2026-05-02
category: best-practices
module: kg
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Domain graph coverage gap depresses corpus match rates below targets
  - Subject extractor surfaces concepts that have no domain entity to match against
  - Adding a new entity type namespace to the KG domain layer
tags:
  - knowledge-graph
  - domain-graph
  - concept-namespace
  - entity-resolution
  - match-rate
  - cross-repo
  - infrastructure
---

# KG domain graph concept namespace expansion for cross-repo and infrastructure coverage

## Context

Post-deploy analysis of mika-arch's multi-corpus KG (mika#877) showed mika-platform corpus resolving at 47.9% and mika-cloud at 31.2%, while the primary mika corpus hit 70.8%. The bottleneck was **domain-graph coverage**: the subject extractor correctly surfaced cross-repo workflow and Helm/K8s infrastructure concepts, but the domain graph only had `skill:*`, `tool:*`, `agent:*`, and `problem_type:*` entities — no targets for these concepts to resolve against.

The fix (mika#928) added a `concept:*` namespace with 20 seed entities across two subcategories: `concept:cross-repo:*` (7 entities) and `concept:infra:*` (13 entities).

## Guidance

**Adding a new entity type to the domain graph requires updates in four places.** The KG pipeline is type-bounded at multiple levels — missing any one produces silent failures (subjects extracted but never resolved):

1. `KG_DOMAIN_ENTITY_TYPES` in `kg_schema.rs` — the single source of truth for domain entity types
2. `APPROVED_ENTITY_TYPES` in `subject_extractor.rs` — controls what entity types the LLM is allowed to extract
3. SQL type filters in `entity_resolver.rs` — 3 hardcoded `AND e.type IN (...)` clauses that select pending subject entities for resolution
4. Seed data in `domain_builder.rs` — the hardcoded entity definitions projected at startup

The resolver's `resolve_single_entity()` uses `KG_DOMAIN_ENTITY_TYPES.contains()` as a gate — types not in the list are classified as "discovered types" and skip resolution entirely. The resolver's `get_domain_candidates()` fetches candidates by type using a range query (`entity_key >= '{type}:' AND entity_key < '{type};'`), so candidates are only presented to the LLM from the same type namespace.

**Concept entities use hierarchical naming: `concept:<category>:<name>`.** This is a deliberate exception to the flat `[a-z0-9_-]+` name convention used by other entity types. The DB CHECK constraint (`entity_key = type || ':' || name`) still holds because `name` includes the subcategory (e.g., `name = "cross-repo:companion-pr-pattern"`). The subcategory is also stored in `properties_json.category` for query convenience.

**Subject entities extracted by the LLM will NOT match the hierarchical domain key exactly.** The subject extractor's validation rejects colons in entity names, so LLM-extracted concept entities will have flat names like `concept:companion-pr-pattern` rather than `concept:cross-repo:companion-pr-pattern`. Stage-1 exact match will miss; Stage-2 LLM disambiguation will present all concept entities as candidates and the LLM picks the correct hierarchical match. This is the expected resolution path — not a bug.

**Seed lists are intentionally hardcoded, not config-driven (KTD-3).** The concepts (Deployment, StatefulSet, companion-PR pattern, etc.) are stable infrastructure terminology with low churn. External config would add deployment complexity for negligible gain. Future additions are a `domain_builder.rs` edit — same workflow as adding a skill or problem_type.

## Why This Matters

Domain-graph coverage is a throughput ceiling for the entity resolver. Adding more resolver capacity (mika#927 fairness fix) or fixing resolver bugs (mika#874 Stage-2 DB fallback) cannot improve match rates for concepts with zero domain-graph targets. Expanding the domain graph is the only way to lift the ceiling for corpora whose primary concepts aren't skills, tools, agents, or problem types.

The four-place update requirement is the main maintenance pitfall. The SQL filters in `entity_resolver.rs` are hardcoded (not derived from `KG_DOMAIN_ENTITY_TYPES`), so adding a sixth domain type requires a grep for the IN-clause pattern. A future refactor to derive the SQL filters dynamically would reduce this to a single-point-of-change.

## When to Apply

- After deploying a new corpus whose docs discuss concepts outside the existing domain graph
- When post-deploy `mika kg status` shows a corpus with resolved/attempted below 60%
- When `kg_resolutions_log` shows high `no_match` rates for a specific subject entity type
- When adding a new category of domain entities (e.g., `concept:security:*` for security-specific concepts)

## Examples

Adding the `concept` type required these four coordinated changes:

```rust
// 1. kg_schema.rs — single source of truth
pub const KG_DOMAIN_ENTITY_TYPES: &[&str] = &[
    "skill", "tool", "agent", "problem_type", "concept"
];

// 2. subject_extractor.rs — LLM extraction approval
pub const APPROVED_ENTITY_TYPES: &[&str] = &[
    "skill", "tool", "agent", "problem_type",
    "solution_path", "failure_mode", "pattern", "concept",
];

// 3. entity_resolver.rs — 3 SQL filters (lines 896, 1032, 1075)
"AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')"

// 4. domain_builder.rs — seed data
const CONCEPT_CROSS_REPO_SEEDS: &[(&str, &str)] = &[
    ("cross-repo:companion-pr-pattern", "Cross-repo PR coordination..."),
    // ... 6 more
];
const CONCEPT_INFRA_SEEDS: &[(&str, &str)] = &[
    ("infra:helm-chart", "Helm chart package..."),
    // ... 12 more
];
```

## Related

- `docs/solutions/best-practices/kg-domain-graph-startup-projection-2026-04-22.md` — sole-writer contract and UPSERT pattern (architectural foundation this feature builds on)
- `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md` — Stage-1/Stage-2 resolution mechanics
- `docs/architecture/kg-id-convention.md` — entity key format documentation (updated with concept namespace)
- mika#928 — the implementing ticket
- mika#877 — the verification milestone that surfaced the coverage gap
- mika#874 — Stage-2 resolver fix (prerequisite)
- mika#927 — per-corpus fairness fix (orthogonal, ships concurrently)
