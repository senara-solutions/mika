---
title: "eval(kg): Provider comparison matrix for KG extraction + resolution"
type: feat
status: active
date: 2026-04-24
issue: 762
---

# eval(kg): Provider comparison matrix for KG extraction + resolution

## Overview

Build a reproducible evaluation harness that compares LLM providers for the two KG call types (entity extraction and entity resolution), producing a decision matrix with concrete quality, cost, and latency numbers. The evaluation runs against real providers using actual `docs/solutions/` documents as inputs.

## Problem Frame

#757 shipped idempotency + budget guards that make cost-per-cycle predictable, but the provider choice — currently `anthropic/claude-haiku-4-5-20251001` (extraction) and `anthropic/claude-sonnet-4-6` (resolution) — was set without evidence. Neither has been quality-compared against cheaper alternatives on real Mika docs. This ticket produces the data that lets the quality-vs-cost trade-off be made rigorously.

## Requirements Trace

- R1. Evaluate minimum provider set: Anthropic Haiku, Anthropic Sonnet, OpenRouter DeepSeek-v3, one mid-tier (Kimi k2.5)
- R2. Measure extraction quality: valid entity/relationship counts, malformed JSON rate, entity name canonicality
- R3. Measure resolution quality: correct match rate against hand-labeled ground truth, confidence calibration, sibling-ambiguity performance
- R4. Measure cost: per-call from actual tokens, projected at 30,400 calls and steady-state scale
- R5. Measure latency: p50 and p95 per call type per provider
- R6. Produce decision matrix document at `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md`
- R7. Commit sample fixtures at `docs/solutions/kg/eval-fixtures-2026-04-24/`
- R8. Reproducible: single command re-runs the evaluation
- R9. Evidence-based recommendation — not defaulted to cheapest or current

## Scope Boundaries

- Evaluation harness code and fixtures only — no changes to production KG code
- No automated provider regression harness (that's the broader Evaluation milestone)
- No embedding-model comparison
- No providers beyond the minimum set unless time permits

### Deferred to Separate Tasks

- Automated regression CI that detects provider quality drift: Evaluation milestone #16
- Provider auto-selection based on eval results: future iteration
- Updating `.env.example` and CLAUDE.md guidance based on recommendation: follow-up PR per issue acceptance criteria

## Context & Research

### Relevant Code and Patterns

- **Extraction:** `SubjectExtractor::extract_document()` in `crates/mika-agent/src/kg/subject_extractor.rs` — builds `[CHUNK N]`-annotated text, sends to LLM via `LlmProvider::send_message()`, parses JSON with structural validation
- **Resolution:** `SubjectEntityResolver::resolve_single_entity()` in `crates/mika-agent/src/kg/entity_resolver.rs` — Stage 1 exact match (free), Stage 2 LLM disambiguation with candidate list
- **Prompt construction:** `build_extraction_prompt()` (private method, line 762) and `build_disambiguation_prompt()` (module-level fn, line 1166) — both return `(String, String)` system/user prompt pairs
- **Provider construction:** `create_real_provider(kind)` in `tests/eval/providers.rs` reads env API keys, constructs via `ModelSpec` + `create_provider()`. For specific models, construct `ModelSpec` directly with desired model name
- **Token usage:** `LlmResponse.usage` carries `input_tokens`/`output_tokens` — directly usable for cost calculation
- **Eval pattern:** `tests/eval/scenarios.rs` defines `ScenarioOutcome` with tokens, latency, pass/fail. Matrix runner in `test_real_provider_matrix.rs` iterates providers × scenarios
- **Calibration:** `CalibrationArtifact` serialized to `target/eval-calibration/` when `MIKA_EVAL_CALIBRATE=1`
- **KG fixtures:** `tests/eval/kg_fixtures/mod.rs` provides `test_db_with_agent()`, `seed_domain_entity()`, `seed_subject_entity()`, `seed_chunk()` — pinned to schema v26
- **Validation constants:** `APPROVED_ENTITY_TYPES` and `APPROVED_RELATIONSHIP_TYPES` are pub — reusable in eval assertions

### Institutional Learnings

- **Valid yield over raw count:** Two-layer validation (prompt instructs, code enforces) means provider comparison should measure post-validation entity yield, not raw extraction count (from `kg-subject-extraction-constrained-ner-2026-04-22.md`)
- **Stage 1 handles 70-80% free:** Focus provider comparison on Stage 2 disambiguation quality — that is where providers differ (from `kg-entity-resolution-two-stage-pipeline.md`)
- **10x cost ratio:** Anthropic direct vs OpenRouter for claude-haiku-4-5 is ~10x for bulk NER. The existing `kg_anthropic_provider` warning reflects this (from `first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`)
- **JSON structural reliability varies by provider:** Kimi tolerates schema issues but produces duplicate blocks; MiniMax has deterministic field name truncation; DeepSeek is strict on schema compliance (from multiple learnings)
- **Hard assertions only, no LLM-judge:** Follow eval convention from #741 — each metric must have objectively checkable signals
- **UTF-8 safety:** All truncation must use `safe_truncate()`, never byte slicing (from #764)
- **No-match bias:** Some providers bias toward forced matching rather than returning null — measure false-positive resolution rate explicitly (from entity-resolution-two-stage-pipeline)

## Key Technical Decisions

- **Direct LLM calls, not SubjectExtractor/SubjectEntityResolver:** The eval calls `provider.send_message()` directly with replicated prompts rather than going through the full extractor/resolver pipeline. Rationale: (1) avoids DB side effects and per-provider DB setup complexity, (2) isolates raw LLM quality from retry/validation logic, (3) the prompts are stable contracts — replicating them in eval is lower-maintenance than making private methods pub. The validation logic is tested separately against the raw LLM output.
- **Provider × model matrix, not ProviderKind defaults:** The issue specifies exact models per provider (e.g., `anthropic/claude-haiku-4-5-20251001`, not just "anthropic"). The eval constructs providers with explicit model names via `ModelSpec`, not `kind.default_model()`.
- **New env var `MIKA_EVAL_KG_PROVIDERS`:** Separate from `MIKA_EVAL_REAL_PROVIDERS` to avoid running KG eval when only running the basic provider matrix. Format: comma-separated `provider/model` strings or `default` for the issue's minimum set.
- **Eval fixtures as committed TOML, not code-generated:** Ground-truth resolution labels and extraction sample doc paths live in committed TOML/JSON files under `docs/solutions/kg/eval-fixtures-2026-04-24/` — reproducible without code changes.
- **Cost from published pricing tables, not API billing:** Per-token costs are hardcoded constants derived from provider pricing pages at eval time, annotated with the date. This avoids requiring billing API access.

## Open Questions

### Resolved During Planning

- **Q: How to handle the private prompt methods?** Resolution: Replicate the prompt text in the eval module. The extraction and disambiguation prompts are stable JSON schema contracts (~60 lines each). Replicating them avoids test-only visibility changes to production code.
- **Q: Which mid-tier provider?** Resolution: `openrouter/moonshotai/kimi-k2.5` — already in use for mika-dev/mika-qa, known to have both strengths (tolerant parsing) and weaknesses (duplicate blocks) worth measuring.
- **Q: How to build resolution ground truth?** Resolution: Hand-label ~30 entity-candidate pairs from actual domain graph entities, including sibling-ambiguity cases. Commit as TOML fixture. The labeler inspects the domain graph built by `domain_builder.rs` and the subject entities from representative docs.

### Deferred to Implementation

- Exact token counts and cost numbers — depend on running the eval
- Whether additional providers merit inclusion after seeing initial results
- Final formatting of the decision matrix document

## Output Structure

```
crates/mika-agent/tests/eval/
  kg_provider_eval/
    mod.rs                         # Module registration + shared types
    prompts.rs                     # Replicated extraction/resolution prompts
    extraction_eval.rs             # Extraction quality scenarios
    resolution_eval.rs             # Resolution quality scenarios
    cost.rs                        # Per-provider pricing constants + projections
    report.rs                      # Decision matrix output formatter

docs/solutions/kg/
  eval-fixtures-2026-04-24/
    extraction_sample_docs.toml    # Paths + expected entity ranges for sample docs
    resolution_ground_truth.toml   # Hand-labeled entity→domain mappings
  kg-provider-evaluation-2026-04-24.md  # Decision matrix (written after eval run)
```

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
For each provider in MIKA_EVAL_KG_PROVIDERS:
  1. Construct provider via ModelSpec + create_provider()
  2. For each sample doc in extraction_sample_docs.toml:
     a. Read doc from disk, build annotated text with [CHUNK N] markers
     b. Build extraction prompt (replicated from SubjectExtractor)
     c. Call provider.send_message(), measure latency
     d. Parse JSON response, run structural validation
     e. Record: entity_count, relationship_count, valid_entity_count,
        json_parse_success, entity_name_quality_score, tokens, latency
  3. For each case in resolution_ground_truth.toml:
     a. Build disambiguation prompt with entity + candidates + context
     b. Call provider.send_message(), measure latency
     c. Parse JSON response
     d. Compare match against ground truth label
     e. Record: correct_match, confidence, false_positive, tokens, latency
  4. Compute aggregates: p50/p95 latency, cost projections, quality scores
  5. Write results to calibration artifact (JSON)

After all providers complete:
  6. Print comparison table to stdout
  7. Optionally write decision matrix markdown
```

## Implementation Units

- [ ] **Unit 1: Eval fixture files — extraction samples + resolution ground truth**

**Goal:** Create the committed fixture files that define what the eval tests against.

**Requirements:** R1, R2, R3, R7

**Dependencies:** None

**Files:**
- Create: `docs/solutions/kg/eval-fixtures-2026-04-24/extraction_sample_docs.toml`
- Create: `docs/solutions/kg/eval-fixtures-2026-04-24/resolution_ground_truth.toml`

**Approach:**
- Select 15 docs from `docs/solutions/` with varying entity density: 5 KG-rich (688, 692, 741, skill-variant, kg-query), 5 architecture-pattern docs (callback-skill, phantom-retry, eval-harness, multi-provider, per-skill-override), 5 low-entity/edge-case docs
- For each doc, record the path and an expected entity count range (min/max) based on manual inspection — not exact assertions, but sanity-check bounds
- Build resolution ground truth: inspect the domain graph entities (skills, tools, agents, problem_types from `domain_builder.rs`), pick 30 entity-candidate pairs including: 10 clear matches, 10 sibling-ambiguous cases (e.g., `self_dev` with candidates `skill:self-dev` and `skill:self-dev-iterate`), 10 no-match cases
- TOML format for machine readability and human editability

**Test expectation:** None — these are data fixtures, not code.

**Verification:**
- Files parse as valid TOML
- Extraction samples reference existing docs that are present on disk
- Resolution ground truth covers the sibling-ambiguity and no-match categories

---

- [ ] **Unit 2: Eval module scaffolding — prompts, types, provider construction**

**Goal:** Create the eval module with replicated prompts, shared types for recording outcomes, and provider construction from `MIKA_EVAL_KG_PROVIDERS`.

**Requirements:** R1, R8

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/mod.rs`
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/prompts.rs`
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/cost.rs`
- Modify: `crates/mika-agent/tests/eval.rs` (register `kg_provider_eval` module)

**Approach:**
- `prompts.rs`: Replicate the extraction system prompt from `build_extraction_prompt()` and the disambiguation system prompt from `build_disambiguation_prompt()`. Include the approved entity/relationship type lists (import from `subject_extractor::APPROVED_ENTITY_TYPES` rather than duplicating). Build annotated text helper that mimics `SubjectExtractor::annotate_text()`.
- `mod.rs`: Define `KgProviderSpec` struct (provider_kind, model_name, display_name). Parse `MIKA_EVAL_KG_PROVIDERS` env var — `default` expands to the 4 issue-specified models; otherwise comma-separated `provider/model` strings. Construct providers via `ModelSpec` + `create_provider()`.
- `cost.rs`: Per-provider per-token pricing constants (input $/1M tokens, output $/1M tokens) dated 2026-04-24. Cost projection functions for 30,400-call burst and steady-state scenarios.
- Register module in `tests/eval.rs` via `mod kg_provider_eval;`

**Patterns to follow:**
- `tests/eval/providers.rs` for env var parsing and provider construction
- `tests/eval/scenarios.rs` for `ScenarioOutcome` struct pattern

**Test scenarios:**
- Happy path: `MIKA_EVAL_KG_PROVIDERS=default` produces 4 provider specs matching the issue's minimum set
- Happy path: Custom `MIKA_EVAL_KG_PROVIDERS=anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3` produces 2 specs
- Edge case: Empty/unset env var returns empty vec (graceful skip)
- Edge case: Cost projection for 0 tokens returns 0
- Happy path: Cost projection for known token counts matches manual calculation

**Verification:**
- Module compiles with `cargo test -p mika-agent --test eval --no-run`
- Provider parsing unit tests pass

---

- [ ] **Unit 3: Extraction evaluation scenarios**

**Goal:** Implement the extraction quality evaluation — read sample docs, build prompts, call providers, validate JSON, score results.

**Requirements:** R2, R4, R5

**Dependencies:** Unit 2

**Files:**
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/extraction_eval.rs`

**Approach:**
- Load fixture paths from `extraction_sample_docs.toml`
- For each provider × doc: read doc from disk, chunk it (simple paragraph-based chunking matching `Chunker` behavior), build annotated text, construct extraction prompt, call `provider.send_message()`, measure wall-clock latency
- Parse JSON response using `serde_json` with the same `ExtractionOutput` struct (it's pub). Run structural validation matching `SubjectExtractor::validate_extraction()` logic — check entity type validity against `APPROVED_ENTITY_TYPES`, relationship constraints, chunk index bounds
- Score entity name quality: check lowercase_underscore format, no colons, canonical naming (e.g., `self_dev` not `Self-Dev`)
- Record per-call: raw entity count, valid entity count, raw relationship count, valid relationship count, json_parse_ok, name_quality_score (fraction of valid names), input_tokens, output_tokens, latency_ms
- Aggregate per provider: total valid entities, total valid relationships, json parse failure rate, mean name quality, p50/p95 latency, total tokens

**Patterns to follow:**
- `tests/eval/test_real_provider_matrix.rs` for the provider × scenario iteration pattern
- `subject_extractor.rs` validation logic (reuse `ExtractionOutput` struct for deserialization)

**Test scenarios:**
- Integration (requires real providers, `#[ignore]`): Run extraction on 3 sample docs per provider, verify JSON parses successfully for at least 2 of 3, verify entity count is within expected bounds from fixture
- Integration: Verify token usage fields are populated (non-zero input_tokens and output_tokens)
- Integration: Verify latency is recorded (> 0ms)

**Verification:**
- Test runs with `cargo test -p mika-agent --test eval -- --ignored kg_provider_eval::extraction` when API keys are set
- Results printed as table to stdout

---

- [ ] **Unit 4: Resolution evaluation scenarios**

**Goal:** Implement the resolution quality evaluation — build disambiguation prompts from ground truth fixtures, call providers, compare against labels.

**Requirements:** R3, R4, R5

**Dependencies:** Unit 2

**Files:**
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/resolution_eval.rs`

**Approach:**
- Load ground truth from `resolution_ground_truth.toml` — each case has: entity_key, entity_confidence, chunk_context, candidates list (with descriptions), expected_match (entity_key or null), case_category (clear_match, sibling_ambiguous, no_match)
- For each provider × case: build disambiguation prompt using replicated `build_disambiguation_prompt()` logic, call `provider.send_message()`, measure latency
- Parse JSON response (`{"match": ... , "confidence": ...}`), compare against expected_match
- Score: correct_match (binary), false_positive (matched when should be null), false_negative (null when should have matched), confidence_when_correct, confidence_when_incorrect
- Slice results by case_category — sibling-ambiguity performance is the key differentiator
- Record per-call: correct, false_positive, confidence, input_tokens, output_tokens, latency_ms
- Aggregate per provider: accuracy overall, accuracy per category, mean confidence calibration gap, p50/p95 latency, total tokens

**Patterns to follow:**
- `entity_resolver.rs` line 1166 for `build_disambiguation_prompt()` structure
- `tests/eval/scenarios.rs` for outcome recording

**Test scenarios:**
- Integration (requires real providers, `#[ignore]`): Run resolution on all 30 ground truth cases per provider, verify at least 70% accuracy on clear_match category
- Integration: Verify no-match cases have at least 60% correct null responses (measures forced-matching bias)
- Integration: Verify sibling-ambiguity cases are measured and reported (even if accuracy varies)
- Integration: Verify confidence values are valid floats in [0.0, 1.0]

**Verification:**
- Test runs with `cargo test -p mika-agent --test eval -- --ignored kg_provider_eval::resolution`
- Results printed as table with per-category breakdown

---

- [ ] **Unit 5: Report generation — decision matrix and calibration artifact**

**Goal:** Combine extraction and resolution results into a unified comparison table, write calibration artifact, and generate the decision matrix document.

**Requirements:** R4, R5, R6, R8, R9

**Dependencies:** Units 3, 4

**Files:**
- Create: `crates/mika-agent/tests/eval/kg_provider_eval/report.rs`

**Approach:**
- Collect all extraction and resolution outcomes per provider
- Compute cost projections using `cost.rs` constants: per-call cost, 30,400-call burst cost, steady-state cost (11 agents × 500 calls/cycle × 5 cycles)
- Format comparison table: rows = providers, columns = extraction quality (valid entities, json reliability), resolution quality (accuracy, sibling accuracy, no-match accuracy), extraction cost, resolution cost, p50 latency, p95 latency
- Write `CalibrationArtifact`-style JSON to `target/eval-calibration/kg-provider-eval-{timestamp}.json`
- Generate markdown decision matrix to stdout — the human copies this into the doc (or a follow-up step writes `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md`)

**Patterns to follow:**
- `tests/eval/calibration.rs` for artifact format
- Issue #762 deliverable section for decision matrix structure

**Test scenarios:**
- Happy path: Given mock extraction/resolution results for 2 providers, report generates valid markdown table
- Happy path: Cost projection with known token counts matches expected dollar amounts
- Edge case: Provider with 100% JSON parse failures still appears in report with quality marked as N/A

**Verification:**
- Full eval run produces both stdout table and calibration artifact
- `cargo test -p mika-agent --test eval -- --ignored kg_provider_eval::full_eval` runs the entire pipeline

---

- [ ] **Unit 6: Decision matrix document + methodology writeup**

**Goal:** After running the eval, write the compound document with methodology, findings, and recommendation.

**Requirements:** R6, R7, R9

**Dependencies:** Units 1-5 (eval results)

**Files:**
- Create: `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md`

**Approach:**
- Run the full evaluation and capture results
- Write the document with sections: Methodology (sample docs, ground truth approach, providers tested, measurement date), Decision Matrix (formatted table), Detailed Findings (per-provider analysis with extraction and resolution breakdowns), Cost Analysis (concrete dollar projections), Recommendation (evidence-based, quality-weighted), Reproducibility (command to re-run)
- Include the recommendation command: `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored kg_provider_eval`

**Test expectation:** None — this is a documentation artifact produced from eval run output.

**Verification:**
- Document exists with all required sections
- Decision matrix has concrete numbers, not placeholders
- Recommendation is grounded in measured data

## System-Wide Impact

- **Interaction graph:** Eval harness is test-only — no production code changes. Touches `tests/eval/` module registration.
- **Error propagation:** Eval failures are test failures, not runtime errors. Provider API errors are caught and recorded as outcomes (not panics).
- **State lifecycle risks:** None — eval uses direct LLM calls without DB writes.
- **API surface parity:** No API changes.
- **Integration coverage:** The eval itself is the integration test — it validates that real providers can handle the KG prompts.
- **Unchanged invariants:** Production `SubjectExtractor` and `SubjectEntityResolver` code is not modified. Prompt replication in eval may drift from production — acceptable for initial eval, tracked as a known maintenance concern.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Replicated prompts drift from production code | Add a comment in both locations referencing the other. If prompts change, the eval needs updating — but prompts are stable (last changed weeks ago) |
| Provider rate limits during eval run | Run providers sequentially, not in parallel. Use conservative delays between calls if needed |
| API keys not available for all 4 providers | Graceful skip per provider — eval reports results for whatever providers have keys. Minimum viable: 2 providers |
| Ground truth labels may be subjective | Document labeling rationale in fixture TOML comments. Include "disputable" flag on ambiguous cases |
| Token costs change over time | Date the pricing constants. Re-running the eval with updated constants is trivial |

## Documentation / Operational Notes

- The eval command is: `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored kg_provider_eval`
- Required env vars: `MIKA_ANTHROPIC_API_KEY`, `MIKA_OPENROUTER_API_KEY` (minimum for the default provider set)
- Expected runtime: ~5-10 minutes (15 docs × 4 providers × extraction + 30 cases × 4 providers × resolution)
- If recommendation is to switch providers: a follow-up PR updates `.env.example`, CLAUDE.md guidance, and #757 compound doc

## Sources & References

- Related issues: #757, #759, #762
- Related code: `crates/mika-agent/src/kg/subject_extractor.rs`, `crates/mika-agent/src/kg/entity_resolver.rs`
- Existing eval patterns: `crates/mika-agent/tests/eval/`
- Learnings: `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`, `docs/solutions/best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md`, `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md`
