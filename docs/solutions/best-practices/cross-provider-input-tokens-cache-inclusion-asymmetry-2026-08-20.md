---
title: "Cross-provider input_tokens semantic asymmetry: Anthropic excludes cache_read, OpenAI-compat includes it"
category: best-practices
date: 2026-08-20
tags: [llm, observability, token-accounting, anthropic, openai-compatible, cache, telemetry, cross-provider]
issue: 1889
problem_type: silent-miscount
module: mika-common::llm, agent_loop
---

# Cross-provider `input_tokens` semantic asymmetry

## Problem

`LlmUsage.input_tokens` carries a **provider-family-specific semantic**, not a
uniform "fresh input" quantity. Any cross-provider aggregation — cost dashboard,
per-turn token accounting, RT-005-shaped estimand computation — that sums or
compares `input_tokens` across families **without applying a per-family
correction** will silently double-count cache reads for OpenAI-compatible
providers.

**The asymmetry, keyed on `LlmProvider::provider_name()`:**

| Provider family | `LlmUsage.input_tokens` semantic | Adapter site |
|---|---|---|
| Anthropic (`anthropic`) | *Fresh* input only — **excludes** cache_read | `mika-common::llm::claude` (native Anthropic mapping) |
| OpenAI-compatible (`openai`, `openrouter`, `groq`, `mistral`, `deepseek`, `zai`, `ollama`) | `prompt_tokens` — **includes** cache_read | `mika-common::llm::openai::from_openai_response` |

For the same conversation shape (36 000-token prompt, 8 192 cache_read), a
naive sum across providers reports:

- Anthropic: `input_tokens = 27 808`, `cache_read_tokens = 8 192` → distinct
  totals, no double-count.
- OpenAI-compat: `input_tokens = 36 000`, `cache_read_tokens = 8 192` →
  `input_tokens` **already contains** the 8 192 cache_read.

Aggregating naively across both — e.g. `SELECT SUM(input_tokens + cache_read_tokens)`
grouped by turn — overcounts every OpenAI-compat turn by its cache_read.

## Where this bites

- **Cost dashboards** aggregating `llm_calls.input_tokens + cache_read_tokens`
  across providers (double-counts cache on the OpenAI-compat rows).
- **Any offline analyzer** consuming the `turn_usage` structured log
  (`mika::otel target`, event `turn_usage` — the mika#1889 RT-005 primary
  measurement channel) that computes an aggregate estimand across mixed-provider
  agent runs.
- **Per-family "cache hit rate"** computed as `cache_read / input_tokens` —
  correct for Anthropic (`hit_rate = cache / (fresh + cache)`), meaningless for
  OpenAI-compat (`hit_rate = cache / prompt`, where `prompt` already includes
  cache).

## Solution

**Two-part contract, applied at every consumer (dashboard, log analyzer, cost
model).** The instrumentation surfaces do **not** normalize (per RT-005 D1
"raw dimensions only, no baked classification"); consumers do.

### 1. Both surfaces MUST carry `provider` (and `model`)

The `llm_calls` DB table already has `provider` + `model` columns. The
`turn_usage` structured log emits `provider = %llm.provider_name()` and
`model = %llm.model_name()` on every event (mika#1889). Consumers that don't
key on `provider` for family selection cannot apply the correction — treating
that field as optional is the silent-miscount vector.

### 2. Consumers apply per-family normalization

Per-turn "fresh input" — the quantity a naive reader thinks `input_tokens`
already is — is:

```
fresh_input = if provider is anthropic:
                input_tokens              // already fresh
              else:                       // openai-compatible family
                input_tokens - cache_read_tokens
```

Or aggregate cost:

```
billable_tokens = if provider is anthropic:
                    input_tokens + cache_read_tokens + output_tokens
                  else:
                    input_tokens + output_tokens    // prompt already includes cache
```

`provider` values to family-map (as of 2026-08-20, from
`mika-common::llm::provider_name()`): `anthropic` → Anthropic family; anything
else known (`openai`, `openrouter`, `groq`, `mistral`, `deepseek`, `zai`,
`ollama`) → OpenAI-compat family; unknown providers should be flagged, not
silently treated as one family.

## Detection

Two greps against `$MIKA_SPIRIT_LOG_FILE` (or the CLI per-agent log at
`~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD`):

```bash
# Confirm Anthropic events have input_tokens ~= fresh (input excludes cache):
grep turn_usage $MIKA_SPIRIT_LOG_FILE | jq 'select(.provider == "anthropic")
  | {step, input_tokens, cache_read_tokens, ratio: (.cache_read_tokens / (.input_tokens + .cache_read_tokens))}'

# Confirm OpenAI-compat events have input_tokens >= cache_read (input includes cache):
grep turn_usage $MIKA_SPIRIT_LOG_FILE | jq 'select(.provider != "anthropic")
  | select(.cache_read_tokens > .input_tokens)'
```

The second query should return **zero rows** — a hit means the provider adapter
regressed and is now emitting a smaller `input_tokens` than `cache_read_tokens`,
inverting the family invariant.

## References

- `crates/mika-common/src/llm/openai.rs` — `from_openai_response`: sets
  `input_tokens = usage.prompt_tokens` (which per the OpenAI response schema
  includes cached-prompt tokens).
- `crates/mika-common/src/llm/claude.rs` — Anthropic adapter: sets
  `input_tokens` from the `usage.input_tokens` field, which the Anthropic API
  documents as fresh input excluding cache.
- `crates/mika-agent/src/agent_loop/mod.rs` — `emit_turn_usage`: emits
  `provider`, `model`, `input_tokens`, `cache_read_tokens`, `cache_write_tokens`
  as raw pass-through; the docstring on `build_turn_usage_fields` names this
  asymmetry as a load-bearing consumer-side invariant.
- mika#479 / `docs/solutions/integration-issues/openai-compatible-provider-cache-token-parsing.md` —
  Coupled learning: the parsing gap (cache_read fields were silently zero for
  OpenAI-compat providers). This document is the sibling gotcha: even after
  the parsing gap is fixed, `input_tokens` still carries a family-specific
  semantic.
- mika#1889 — RT-005 physics pilot brick 4/5 (the `turn_usage` instrumentation
  that surfaced the need for `provider` on the log surface as the primary
  measurement channel).
- Prime hard condition #1 (planning-tokens SEUL as primary outcome) —
  motivates the raw-emit-and-normalize-offline discipline that makes this
  consumer-side contract the correct shape.
