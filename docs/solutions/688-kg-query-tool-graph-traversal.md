---
module: crates/mika-agent/src/kg
tags: [knowledge-graph, query, traversal, recursive-cte, cycle-detection, sqlite]
problem_type: feature-implementation
date: 2026-04-22
issue: 688
---

# KG Query Tool: Graph Traversal for Agent Self-Knowledge

## Problem

The knowledge graph had three populated layers (domain, lexical, subject) plus entity resolution bridging subject to domain, but no agent-facing read interface. Agents couldn't ask "what solves CI failures?" and get a traversal from `problem_type:ci_failure` through `SOLVED_BY` to solution paths and skills.

## Solution

Added `query_knowledge_graph` as a builtin tool with two query modes:

1. **Free-text question** — hybrid entry-path resolution finds starting entities via three parallel strategies (direct domain match, subject match, semantic chunk search), then traverses.
2. **Direct traversal** — caller provides a known `entity_key`, skipping entry resolution.

### Key Design Decisions

- **Parallel entry paths over linear flow** — A single search→resolve→traverse pipeline has a single point of failure at each stage. Running paths A (domain name match), B (subject name match), and C (semantic chunk search) in parallel, then merging with dedup, provides resilience when any one path misses.

- **Annotate, don't filter** — Agent-scoped queries enrich skill entities with `agent_context.enabled` metadata but do NOT filter disabled skills from results. The LLM decides intent ("what skills do I have?" vs "what skills could I enable?"); the tool provides data.

- **Three-value status** — `ok`, `starting_entity_missing`, `traversal_empty` distinguish "no entities found at all" from "entity exists but has no relationships." The self-knowledge skill (#692) uses this for fallback logic.

- **Compact default, opt-in context** — Results return entity keys, edge types, and confidence by default. Chunk prose text requires `include_context: true`. Keeps context window impact predictable.

## Lessons Learned

### Cycle detection in recursive CTEs requires delimiter-bounded matching

The initial implementation used `INSTR(t.path, CAST(r.to_entity_id AS TEXT)) = 0` to detect cycles in the traversal path. This has a substring false-positive bug: entity ID `2` would be incorrectly blocked if the path contained `12` (because `INSTR("12", "2") != 0`).

**Fix:** Use delimiter-bounded matching: `INSTR(',' || t.path || ',', ',' || CAST(r.to_entity_id AS TEXT) || ',') = 0`. This wraps both the path and the search term in commas, ensuring only whole-number matches.

**Test:** Added a regression test seeding entities with IDs 11, 12, and 2, with edges 11→12→2. The naive INSTR check blocks entity 2 because path "11,12" contains substring "2". The delimiter-bounded check correctly allows it.

### TraversalEmpty detection must count hops, not entities

Comparing `traversed.len() <= starting_entities.len()` to detect "no edges found" breaks when cross-layer dedup collapses starting entities. For example, if the same conceptual entity appears in both domain and subject layers, dedup in `traverse_graph` reduces the count. The correct check is `traversed.iter().any(|e| e.hop > 0)` — if any entity was reached via an edge, the traversal found something.

### Dead API fields are worse than missing features

The initial implementation included `cross_layer: bool` in the tool schema but never read it. An LLM seeing this parameter in the tool definition would naturally try to use it, getting silent no-op behavior. Removed during review — add back only when the cross-layer hop logic is implemented.

## Files Changed

- `crates/mika-agent/src/kg/query.rs` — Query engine: entry paths, recursive CTE traversal, ranking, agent context enrichment
- `crates/mika-agent/src/tools/query_knowledge_graph.rs` — Tool wrapper: input parsing, validation, output formatting
- `crates/mika-agent/src/tools/mod.rs` — Tool registration
- `crates/mika-agent/src/kg/mod.rs` — Module export

## Related

- [`workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — KG milestone retrospective; #688 is where the silent callback failure occurred (attributed to kimi-k2.5 conflating relay events with conversational turns; motivates `mika#721`).
- [`692-self-knowledge-kg-upgrade.md`](692-self-knowledge-kg-upgrade.md) — consumer of this tool; uses the three-value status for fallback routing.
- [`best-practices/kg-entity-resolution-two-stage-pipeline.md`](best-practices/kg-entity-resolution-two-stage-pipeline.md) — resolution edges this traversal uses for cross-layer hops.
