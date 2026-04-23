# KG Self-Knowledge Eval Scenarios (#740)

Seven scenarios verifying that agents correctly query the knowledge graph before claiming capability or state.

## Running

```bash
# Unit tier (mock LLM, every CI push)
cargo test -p mika-agent --test eval kg_self_knowledge

# Integration tier (real providers, opt-in)
MIKA_EVAL_REAL_PROVIDERS=anthropic MIKA_KG_RESOLUTION_MODEL=anthropic/claude-haiku-4-5-20251001 \
  cargo test -p mika-agent --test eval kg_self_knowledge -- --ignored
```

## Fixture Helpers

Shared helpers live at `tests/eval/kg_fixtures/mod.rs` (crate-shared, not nested here). Available helpers:

| Helper | Seeds into | Returns |
|--------|-----------|---------|
| `seed_domain_entity(db, spec)` | `kg_entities` | row ID |
| `seed_domain_relationship(db, from, to, type)` | `kg_relationships` | row ID |
| `seed_subject_entity(db, spec)` | `kg_subject_entities` | row ID |
| `seed_chunk(db, spec)` | `kg_chunks` + `search_content` | chunk ID |
| `seed_chunk_subject(db, chunk_id, subject_id)` | `kg_chunk_subjects` | row ID |
| `seed_resolution(db, subject_id, domain_id, conf, outcome)` | `kg_subject_resolutions` + `kg_resolutions_log` | — |
| `disable_skill(db, agent_id, skill_name)` | `skill_overrides` | — |

Fixtures are pinned to schema v25. On schema bump, `assert_schema_version()` fails with an actionable message.

## Tag Vocabulary (`self-knowledge:*`)

| Tag | Trigger Condition |
|-----|-------------------|
| `self-knowledge:query-invoked` | Agent called `query_knowledge_graph` before final response |
| `self-knowledge:capability-claimed-without-query` | Agent claimed capability state without a KG query |
| `self-knowledge:stage-1-skipped` | Stage 1 exact match bypassed unexpectedly |
| `self-knowledge:agent-context-missing` | Result returned but `agent_context` annotation absent/wrong |
| `self-knowledge:disambiguation-correct` | Stage 2 picked the structurally-expected candidate |
| `self-knowledge:disambiguation-plausible-alternative` | Stage 2 picked a different-but-defensible candidate |

### Scope Boundary with #741 (`grounding:*`)

`self-knowledge:*` tags cover the code path from tool-invocation through resolver. Agent *response quality* given a successful KG result (e.g., "KG returned correct result but agent ignored it") is a **grounding failure** routed to `#741`'s `grounding:*` namespace. Tags attribute to **cause-location**, not symptom.

## Capability x Status Matrix

| # | Scenario | `ok` | `starting_entity_missing` | `traversal_empty` | `matched_exact` | `matched_llm` | `skipped_no_llm` | `agent_context` |
|---|----------|:----:|:-------------------------:|:------------------:|:---------------:|:--------------:|:----------------:|:---------------:|
| 1 | tool_selection_query_knowledge_graph | | | | | | | |
| 2 | path_a_direct_domain_match | X | | | | | | |
| 3 | path_b_subject_match_agent_scoped | X | X | | | | | |
| 4 | path_c_semantic_via_chunks | X | | | | | | |
| 5 | stage_1_exact_match | | | | X | | X | |
| 6 | stage_2_llm_disambiguation | | | | | X | X | |
| 7 | agent_context_annotation_disabled | X | | | | | | X |

Legend: `X` = scenario asserts this status outcome.

## How to Add a Scenario

1. Create `crates/mika-agent/tests/eval/kg_self_knowledge/<name>.rs`
2. Add `pub mod <name>;` to `mod.rs`
3. Use `kg_fixtures::*` for seeding
4. Call `assert_schema_version(&db).await;` in setup
5. Add hard assertions (regression-gating) and optional soft tags
6. Update this README's matrix

## Architecture References

- KG tables: `crates/mika-agent/CLAUDE.md` (Knowledge Graph section)
- Entity key format: `docs/architecture/kg-id-convention.md`
- Domain builder: `crates/mika-agent/src/kg/domain_builder.rs`
- Query engine: `crates/mika-agent/src/kg/query.rs`
- Entity resolver: `crates/mika-agent/src/kg/entity_resolver.rs`
