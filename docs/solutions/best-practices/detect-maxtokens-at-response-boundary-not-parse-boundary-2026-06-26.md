---
title: "Detect MaxTokens truncation at the response boundary, not the parse boundary"
date: 2026-06-26
category: best-practices
module: mika-agent
problem_type: best_practice
component: llm
severity: medium
applies_when:
  - An LLM call parses structured output (JSON) and falls back to a semantic retry on parse failure
  - A model hits its output-token cap and returns a truncated body instead of an error
  - A previously-silent extraction/parse failure is suspected of burning repeated doomed LLM calls
tags:
  - llm
  - kg
  - subject-extractor
  - max-tokens
  - retry
  - observability
issue: mika#1091
---

# Detect MaxTokens truncation at the response boundary, not the parse boundary

## Context

The KG subject extractor (`crates/mika-agent/src/kg/subject_extractor.rs`,
`call_llm_with_retry`) parses the LLM response as JSON and, on parse failure,
runs one **semantic retry** with prompt reinforcement (C2.2). That retry is the
correct recovery for a model that emitted *malformed* JSON.

It is the wrong recovery for a model that emitted *truncated* JSON. When
`openrouter/openai/gpt-5-nano` hit its output-token cap, the provider returned
`Ok(response)` with an incomplete body — **not** an `Err`
(`finish_reason: "length"` maps to `LlmStopReason::MaxTokens` at
`crates/mika-common/src/llm/openai.rs`; Claude's `StopReason::MaxTokens` maps to
the same). The truncation was invisible to the retry taxonomy, so it was
mishandled as a semantic failure: the code fell through to
`retry_with_reinforcement`, which re-sends the truncated output **plus** a
reinforcement prompt against the **same** `max_tokens` budget — guaranteed to
truncate again — ending in `extraction_semantic_exhausted` → `Ok(None)` →
subjects silently dropped from the knowledge graph.

It compounded: the `Ok(None)` path returns `ExtractionStats::default()` *before*
`write_extraction_results` writes the `kg_extractions` idempotency marker, so a
truncating doc stays pending and burns **two** doomed LLM calls on every 30-min
extraction tick, indefinitely — a silent, self-perpetuating budget drain with no
operator signal.

## Guidance

Check `response.stop_reason` **before** attempting to parse. A length/MaxTokens
truncation is a distinct failure class from malformed output and must short-circuit:

```rust
// after a successful send_message, before parse_extraction_json:
let stop_reason = response.stop_reason;
let text = response.text_content();

if stop_reason == LlmStopReason::MaxTokens {
    warn!(
        trace_id = %self.trace_id,
        event = "extraction_max_tokens_truncated", // distinct, greppable
        output_len = text.len(),
        "LLM hit MaxTokens — response truncated, skipping parse and retry (log-and-skip per C2.3)"
    );
    return Ok(None);
}
// ... otherwise parse, and on parse failure run the semantic retry
```

Apply the check at **every** parse site (the extractor has two: the attempt-1
success branch and the transport-retry success branch). Also thread the
first-attempt `stop_reason` into the semantic-retry path and log it on
`extraction_semantic_exhausted`, so any exhaustion that still occurs can be
attributed to truncation vs. genuine malformed JSON.

## Why This Matters

- **A retry that re-sends content against the same token budget is doomed by
  construction.** Detecting truncation at the response boundary skips the wasted
  call (halves per-cycle cost for truncating docs) instead of discovering the
  failure one expensive round-trip later.
- **A distinct telemetry event turns a silent failure observable.** Reusing the
  generic `extraction_semantic_exhausted` event hid a self-perpetuating drain;
  `extraction_max_tokens_truncated` is greppable and points straight at the cause.
- **Stop-reason is the only reliable truncation signal.** A truncated JSON body
  is often *almost* valid — string-tolerant parsers can mask the problem or, worse,
  parse a partial object. The provider's stop-reason is authoritative; the byte
  stream is not.

## When to Apply

Any LLM-calling code with a **parse-then-semantic-retry** shape, across providers.
The mapping is provider-agnostic in this codebase — both Claude
`StopReason::MaxTokens` and OpenAI/OpenRouter `finish_reason: "length"` normalize
to `LlmStopReason::MaxTokens` — so the boundary check is written once against the
normalized enum and covers every provider.

## Scope Note

This fix is deliberately **additive** and preserves stay-pending semantics: the
MaxTokens path still leaves the doc pending (no idempotency marker written),
because truncation can be transient if the doc, the injected roster, or the model
changes. *Eliminating* the perpetual re-attempt (raising the extraction
output budget, switching model, chunking inputs, or pausing the path) is a
separate strategic decision, deferred to the operator — the telemetry shipped
here is the data needed to make it.

## Related

- `docs/solutions/architecture-patterns/kg-extraction-trigger-semantics-2026-05-09.md` — when extraction runs and the idempotency-marker contract.
- `docs/solutions/kg/truncation-decision-2026-06-26.md` (mika#766) — distinct concern: empirical decision on *input-corpus* truncation, not LLM *response* truncation.
