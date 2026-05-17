---
module: kg/subject_extractor
tags: [kg, extraction, roster, phantom-entities, validation, prompt-engineering]
problem_type: phantom_entity_production
category: architecture-patterns
ticket: mika#1158
related: [mika#1154, mika#1152]
---

# Roster Grounding: Closed-Set Constraint for KG Subject Extraction

## Problem

The KG subject extractor was roster-blind — `build_extraction_prompt()` handed
the LLM only an approved-types list with no reference to the actually-canonical
entities in `kg_entities`. This produced phantom subjects like `agent:vincent`,
`agent:ci`, `agent:tower_http`, `tool:tailwind` that had no domain-graph
counterpart and polluted `kg_subject_entities` with unresolvable rows.

The resolver-sonnet baseline experiment (mika#1152) found 0/87 sampled
`no_match` outcomes had a domain-graph match — 100% phantom rate on sampled
`no_match` entries.

## Solution

**Inject the live domain-entity roster into the extraction prompt** and add a
`discovered: true` carveout for clearly-named non-roster entities.

### Architecture

```
domain_builder (boot) → kg_entities (roster source)
                              ↓
SubjectExtractor::load_roster_snapshot() [once per batch]
                              ↓
build_extraction_prompt() ← rendered roster section
                              ↓
validate_extraction_output() ← roster constraint check
                              ↓
kg_subject_entities.discovered / discovery_reason [storage]
                              ↓
entity_resolver → skipped_discovered_subject [outcome]
```

### Key Design Decisions

1. **Soft prior, not hard filter.** Non-roster entities are allowed via
   `discovered: true` + `discovery_reason`. This preserves signal about
   entities that should be promoted to the domain graph while keeping the
   sole-writer contract intact.

2. **Roster-constrained types exclude `concept`.** Concept names use
   hierarchical colons (`cross-repo:companion-pr-pattern`) which collide
   with the existing colon-rejection validator rule. Deferred to a follow-up.

3. **Empty-roster = extraction refusal.** When the roster cannot be trusted
   (UnbuiltGraph or EmptyRosterTypes), extraction skips the entire batch
   rather than running in lenient mode. This is safer than silently accepting
   phantoms during a transient boot window.

4. **Roster mismatch = log + drop.** Non-roster non-discovered entities are
   dropped at validation with a `WARN subject_roster_mismatch` event. The
   entity data is intentionally lost (it's wrong by definition). The log
   event preserves audit signal for model/prompt tuning.

5. **Per-batch roster fetch.** The roster is loaded once per `extract_pending`
   batch (not per-doc). The cost is one indexed query + one `COUNT(*)` per
   batch. The roster is stale between server boots (domain_builder runs once
   at boot), which is acceptable for this eventual-consistency surface.

### RosterLoadState Discriminant

```rust
pub enum RosterLoadState {
    Populated,       // N≥1 roster entries — proceed
    UnbuiltGraph,    // kg_entities empty — domain_builder hasn't run
    EmptyRosterTypes // kg_entities has rows but zero roster types
}
```

The discriminant is resolved by a single cheap `COUNT(*)` query when the
roster-types query returns zero rows.

### Schema Changes

- v35→v36: `kg_subject_entities.discovered INTEGER NOT NULL DEFAULT 0` +
  `kg_subject_entities.discovery_reason TEXT` (additive ALTER TABLE)
- v36→v37: `kg_resolutions_log.outcome` CHECK widened to include
  `'skipped_discovered_subject'` (table rebuild)

### Resolver Integration

Discovered subjects (`discovered=1`) skip resolution entirely via
`ResolutionResult::SkippedDiscoveredSubject`. They never enter `kg_subject_resolutions`
(no domain edge created) and log `outcome='skipped_discovered_subject'` in
`kg_resolutions_log`. This preserves the sole-writer contract: no code path
writes `kg_entities` rows for discovered subjects.

## Operational Signals

- `extraction_roster_loaded` (INFO): roster fetched successfully, shows entry count
- `extraction_roster_unbuilt` (WARN): kg_entities empty, batch skipped
- `extraction_roster_failed` (ERROR): kg_entities has rows but zero roster types
- `subject_roster_mismatch` (WARN): per-entity, LLM ignored roster directive

## Reuse Pattern

The "closed-set roster as prompt constraint + discovered carveout" pattern is
reusable for any LLM extraction where the valid output set is known but may
need extension. The `discovered` flag preserves signal without auto-promoting
into the canonical set (which would violate sole-writer contracts in a
multi-writer system).
