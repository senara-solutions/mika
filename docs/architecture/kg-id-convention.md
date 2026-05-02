# KG Entity ID Convention

**Date:** 2026-04-21
**Status:** Active
**Component:** mika-agent/db (Knowledge Graph)

## Format

```
<type>:<name>
```

- `<type>` — lowercase entity type from the reserved set (see below).
- `<name>` — lowercase identifier, typically `[a-z0-9_-]+`. Concept entities use hierarchical names with embedded colons (e.g., `cross-repo:companion-pr-pattern`); other types use flat names without colons.
- Separator is a single colon (`:`) between type and name.

Examples: `skill:self-dev`, `tool:run_claude_pilot`, `agent:mika-dev`, `problem_type:fabrication`, `concept:infra:helm-chart`.

## Reserved Domain Entity Types

The canonical source of truth is `KG_DOMAIN_ENTITY_TYPES` in `crates/mika-agent/src/db/kg_schema.rs`. The current set:

| Type | Derivation Rule | Example |
|------|----------------|---------|
| `skill` | `skill:<skill.toml name>` | `skill:self-dev` |
| `tool` | `tool:<registered tool name>` | `tool:run_claude_pilot` |
| `agent` | `agent:<agent name from config>` | `agent:mika-dev` |
| `problem_type` | `problem_type:<slug>` | `problem_type:fabrication` |
| `concept` | `concept:<category>:<name>` | `concept:infra:helm-chart` |

To add a new domain type: update `KG_DOMAIN_ENTITY_TYPES` in `kg_schema.rs` first, then update this document.

### Concept Entity Naming

Concept entities use hierarchical `<category>:<name>` in the name field, producing three-segment entity keys: `concept:<category>:<name>`. Two categories are currently defined:

| Category | Coverage | Example entity keys |
|----------|----------|-------------------|
| `cross-repo` | Cross-repo workflow patterns, coordination primitives, worktree management | `concept:cross-repo:companion-pr-pattern`, `concept:cross-repo:worktree-management` |
| `infra` | Helm charts, Kubernetes resources, cloud topology, provisioning | `concept:infra:helm-chart`, `concept:infra:kubernetes-deployment`, `concept:infra:aws-eks` |

The `concept` type is seeded by `domain_builder.rs` from hardcoded constants (`CONCEPT_CROSS_REPO_SEEDS`, `CONCEPT_INFRA_SEEDS`). To add new concept entities, update the seed lists in `domain_builder.rs`. To add a new concept category, add a new seed list following the same pattern.

The hierarchical naming is an exception to the general `[a-z0-9_-]+` name convention. The DB CHECK constraint (`entity_key = type || ':' || name`) still holds because `name` is simply everything after the first colon. The category is stored redundantly in `properties_json.category` for query convenience.

## Subject-Layer Entities

Subject-layer entities (`kg_subject_entities`) use the same `<type>:<name>` format but are scoped per-agent via the `(agent_id, entity_key)` UNIQUE constraint. The LLM extractor chooses the type:

- If the mention resolves to a domain entity, use the domain type (e.g., `skill:self-dev`).
- If the mention is subject-only, use a subject type (e.g., `failure_mode:oom`, `solution_path:restart-loop`).

Subject types are not restricted to `KG_DOMAIN_ENTITY_TYPES`. The extractor may introduce new types as needed.

## Schema Enforcement

- `kg_entities.entity_key` has a CHECK constraint: `CHECK (entity_key = type || ':' || name)`.
- `kg_subject_entities.entity_key` has the same CHECK constraint.
- The `format_entity_key(kind, name)` helper in `kg_schema.rs` produces the canonical format. All entity creation code should use this helper.

## Storage

- `kg_entities.id` (INTEGER PRIMARY KEY AUTOINCREMENT) is the internal join key. Foreign keys in `kg_relationships` and `kg_subject_resolutions` reference this.
- `kg_entities.entity_key` (TEXT UNIQUE) is the external-facing identifier for manifests, logs, and user/LLM queries.
- Renames update `entity_key` in one row — no FK cascade needed.

## Cross-Layer Queries

When querying across domain and subject layers, disambiguate by source table, not by key alone. A subject entity key `skill:self-dev` in `kg_subject_entities` is a per-agent mention, not the domain entity. Resolution edges in `kg_subject_resolutions` link subject mentions to domain entities.

## Precedent

The `<type>:<id>` format extends the existing `audit_events.target_key` convention (`person:42`, `task:<uuid>`, `skill:<name>`), formalized here for the KG.
