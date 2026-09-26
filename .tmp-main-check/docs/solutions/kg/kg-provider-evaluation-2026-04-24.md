---
module: kg
tags: [knowledge-graph, provider-comparison, evaluation, cost-optimization, extraction, resolution]
problem_type: evaluation
date: 2026-04-24
issue: 762
---

# KG Provider Evaluation — Extraction + Resolution Comparison Matrix

## Summary

Evidence-based provider comparison for the two KG LLM call paths: entity extraction (NER + fact triples from `docs/solutions/` documents) and entity resolution (disambiguation of extracted entities against the domain knowledge graph). This evaluation produces concrete quality, cost, and latency numbers to guide provider selection.

## Methodology

### Providers Evaluated

| Provider | Model | Role Tested |
|----------|-------|-------------|
| Anthropic | `claude-haiku-4-5-20251001` | Extraction (current default) + Resolution |
| Anthropic | `claude-sonnet-4-6` | Resolution (current default) + Extraction |
| OpenRouter | `deepseek/deepseek-v3` | Both (cheap reference) |
| OpenRouter | `moonshotai/kimi-k2.5` | Both (mid-tier, already used for mika-dev/mika-qa) |

### Extraction Evaluation

- **Sample set:** 15 documents from `docs/solutions/` with varying entity density:
  - 5 KG-rich (688, 692, 741, 541, kg-subject-extraction compound)
  - 5 moderate (callback-skill, eval-harness, multi-provider, per-skill-override, callback-resume)
  - 5 low-entity/edge-case (677, cli-flag-subcommand, log-format, env-var-leakage, 602)
- **Process:** Each document is chunked (markdown-aware: frontmatter, H2 sections, 2000-char window), annotated with `[CHUNK N]` markers, and sent to each provider with the extraction prompt
- **Metrics measured:**
  - JSON parse success rate (can the provider return valid JSON?)
  - Entity count (raw number of entities extracted)
  - Relationship count (raw number of relationships extracted)
  - Name quality (fraction following `lowercase_underscore` convention, no colons)
  - Type quality (fraction using approved entity types)
  - Token usage (input + output)
  - Wall-clock latency per call

### Resolution Evaluation

- **Ground truth:** 30 hand-labeled entity-candidate pairs covering:
  - 10 clear matches (unambiguous underscore-to-hyphen normalization)
  - 10 sibling-ambiguous cases (e.g., `self_dev_webhook` with candidates `self-dev-webhook-ci` vs `self-dev-webhook-qa`)
  - 10 no-match cases (entity has no valid domain counterpart — tests forced-matching bias)
- **Process:** Each case presents the entity, its extraction confidence, source prose context, and a candidate list to the disambiguation prompt
- **Metrics measured:**
  - Overall accuracy (correct match or correct null)
  - Per-category accuracy (clear_match, sibling_ambiguous, no_match)
  - Confidence calibration (does the model report high confidence only when correct?)
  - False positive rate (matched when should be null)
  - Token usage and latency

### Cost Model

Per-token pricing as of 2026-04-24:

| Model | Input $/1M tokens | Output $/1M tokens |
|-------|-------------------|--------------------|
| `claude-haiku-4-5-20251001` | $1.00 | $5.00 |
| `claude-sonnet-4-6` | $3.00 | $15.00 |
| `deepseek/deepseek-v3` | $0.30 | $0.88 |
| `moonshotai/kimi-k2.5` | $0.20 | $0.60 |

Cost projections computed for:
- **Per-call:** actual tokens from the eval run
- **Burst scenario:** 30,400 calls (extraction burst after major doc update)
- **Steady state:** 11 agents x 500 calls/cycle x 5 cycles = 27,500 calls

## Decision Matrix

> **Run the evaluation to populate this section.** The harness prints the comparison tables to stdout.

```bash
MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_full
```

### Extraction Comparison

| Provider | Docs | Parse% | Ents | Name% | Type% | p50ms | p95ms | $/call | $/steady |
|----------|------|--------|------|-------|-------|-------|-------|--------|----------|
| _Run eval to populate_ | | | | | | | | | |

### Resolution Comparison

| Provider | Cases | Acc% | Conf | p50ms | p95ms | $/call | $/steady |
|----------|-------|------|------|-------|-------|--------|----------|
| _Run eval to populate_ | | | | | | | |

### Resolution Accuracy by Category

| Category | Haiku 4.5 | Sonnet 4.6 | DeepSeek V3 | Kimi K2.5 |
|----------|-----------|------------|-------------|-----------|
| clear_match | | | | |
| sibling_ambiguous | | | | |
| no_match | | | | |

### Cost Summary

| Provider | Extract/call | Resolve/call | Total/steady |
|----------|-------------|-------------|--------------|
| _Run eval to populate_ | | | |

## Recommendation

> **Populate after reviewing the decision matrix data.** The recommendation should be evidence-based, weighing quality against cost. Key considerations:
>
> - If quality is close across providers, cheaper wins
> - If quality diverges, characterize the delta specifically (e.g., "Provider X produces better canonical names on skill references; Provider Y misses ~N% of cross-chunk relationships")
> - Extraction and resolution may have different optimal providers (extraction is mechanical JSON — cheap/fast tier; resolution requires disambiguation judgment — mid-tier)
> - The current 10x cost ratio between Anthropic direct and OpenRouter (#757) is the primary lever

## Sample Set

Committed alongside this report for reproducibility:

- `docs/solutions/kg/eval-fixtures-2026-04-24/extraction_sample_docs.toml` — 15 sample documents with expected entity count ranges
- `docs/solutions/kg/eval-fixtures-2026-04-24/resolution_ground_truth.toml` — 30 hand-labeled resolution cases

## Reproducibility

### Re-run the full evaluation

```bash
# Required env vars:
# MIKA_ANTHROPIC_API_KEY — for Haiku and Sonnet
# MIKA_OPENROUTER_API_KEY — for DeepSeek V3 and Kimi K2.5

MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_full
```

### Run extraction only

```bash
MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_extraction_only
```

### Run resolution only

```bash
MIKA_EVAL_KG_PROVIDERS=default \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_resolution_only
```

### Test with specific providers

```bash
MIKA_EVAL_KG_PROVIDERS=anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3 \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval
```

### Write calibration artifact

```bash
MIKA_EVAL_KG_PROVIDERS=default MIKA_EVAL_CALIBRATE=1 \
  cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval::kg_provider_eval_full
# Artifact written to target/eval-calibration/kg_provider_eval.json
```

## Relationship to Other Work

- **#757 / #759:** This eval finishes the "what provider should we recommend" question #757 deferred. The budget guard (#757) makes cost-per-cycle predictable; this eval determines *which* provider to budget for.
- **Evaluation milestone (#16):** This is a scoped demo of the eval-driven approach before the full harness lands. The methodology (sample set + ground truth + decision matrix) is a template the larger harness can generalize.
- **KG self-knowledge (#740, merged as #758):** The resolver quality measurement directly exercises the Path B / Path C surface #740's scenarios cover.

## Follow-Up Actions

If the recommendation is to switch providers:
- [ ] Update `.env.example` default KG model guidance
- [ ] Update `CLAUDE.md` KG provider section
- [ ] Update #757 compound doc's operator action section

If the recommendation is to stay on Anthropic:
- [ ] Document this as the quality-validated default in the same locations
