---
title: "KG provider eval harness — reproducible comparison methodology"
date: 2026-04-24
category: best-practices
module: kg
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Comparing LLM providers for a specific task (extraction, resolution, classification)
  - Building evidence for provider selection decisions
  - Establishing cost-quality trade-offs for bulk LLM operations
tags: [knowledge-graph, provider-comparison, evaluation, cost-optimization, llm, extraction, resolution, testing]
---

# KG provider eval harness — reproducible comparison methodology

## Context

The KG subsystem uses two LLM call types — entity extraction (NER + fact triples from `docs/solutions/` documents) and entity resolution (disambiguation of extracted entities against the domain graph). Both defaulted to Anthropic models without evidence-based comparison against alternatives. After #757 shipped budget guards making cost-per-cycle predictable, the natural follow-up was to determine which provider should be budgeted for.

Prior to this eval harness, provider selection was based on "it works" rather than measured quality-cost-latency trade-offs. The ~10x cost ratio between Anthropic direct and OpenRouter for bulk NER (documented in `first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`) motivated the comparison, but cost alone shouldn't drive the decision — quality matters.

## Guidance

### Architecture: Direct LLM calls, not production pipeline

The eval calls `provider.send_message()` directly with replicated prompts rather than going through `SubjectExtractor`/`SubjectEntityResolver`. This isolates raw LLM quality from retry logic, DB side effects, and validation layers.

The extraction and disambiguation prompts are replicated in `crates/mika-agent/tests/eval/kg_provider_eval/prompts.rs`, not imported from production code. The production prompt builders are private methods on internal structs — making them `pub(crate)` for testing would be a larger change than replicating ~60 lines of stable prompt text.

### Fixtures: Committed TOML, not code-generated

Ground truth lives in `docs/solutions/kg/eval-fixtures-2026-04-24/`:
- `extraction_sample_docs.toml` — 15 docs from `docs/solutions/` across 3 density categories (kg_rich, moderate, low_entity) with expected entity count ranges
- `resolution_ground_truth.toml` — 30 hand-labeled entity-candidate pairs across 3 categories (clear_match, sibling_ambiguous, no_match)

TOML format enables human review and editing without touching Rust code. The `disputable` flag on resolution cases marks labels where reasonable annotators might disagree.

### Provider construction: Explicit models, not defaults

The eval uses `MIKA_EVAL_KG_PROVIDERS` (separate from `MIKA_EVAL_REAL_PROVIDERS`) to avoid running KG eval during basic provider matrix tests. Format: comma-separated `provider/model` strings or `default` for the four-provider minimum set. Providers are constructed via `ModelSpec` with explicit model names, not `ProviderKind::default_model()`.

### Quality metrics: Valid yield, not raw count

Following the institutional learning from `kg-subject-extraction-constrained-ner-2026-04-22.md`, extraction quality is measured by post-validation entity yield:
- **Name quality:** fraction of entities following `lowercase_underscore` convention (no colons, no spaces)
- **Type quality:** fraction using approved entity types from `APPROVED_ENTITY_TYPES`
- **JSON reliability:** parse success rate (measures structural response quality)

Resolution quality measures:
- **Per-category accuracy:** clear_match, sibling_ambiguous, no_match reported separately
- **False-positive rate:** matched when should be null (measures forced-matching bias)
- **Confidence calibration:** does the model report high confidence only when correct?

### Cost model: Published pricing, not API billing

Per-token costs are hardcoded constants in `cost.rs`, dated and sourced from provider pricing pages. This avoids requiring billing API access and makes the eval self-contained. Projections cover per-call, 30,400-call burst, and steady-state (11 agents x 500 calls/cycle x 5 cycles).

## Why This Matters

Without measured evidence, provider selection defaults to either "cheapest" (risks quality regression) or "current" (misses cost savings). The eval harness provides:
1. Reproducible numbers for quality-cost trade-offs
2. Per-category resolution accuracy (the sibling-ambiguity cases are where providers actually differ)
3. Concrete dollar projections at operational scale
4. A template for future provider evaluations (embedding models, agent loop providers)

## When to Apply

- When selecting or changing LLM providers for any bulk/pipeline task
- When pricing changes make a previously-expensive provider competitive
- When a new model release warrants re-evaluation
- When the KG prompts change (re-run to verify provider quality holds)

## Examples

Run the full evaluation:
```bash
MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_full
```

Run extraction only:
```bash
MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_extraction_only
```

Test with specific providers:
```bash
MIKA_EVAL_KG_PROVIDERS=anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3 \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval
```

## Related

- #762 — This issue
- #757 / #759 — Budget guard and idempotency that motivated the eval
- `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — Anthropic vs OpenRouter cost ratio
- `docs/solutions/best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md` — Valid yield measurement approach
- `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md` — Stage 2 disambiguation quality focus
- `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md` — Decision matrix document (populated after running the eval)
