---
title: "Langfuse receiving non-LLM infrastructure spans and missing LLM generations"
category: integration-issues
date: 2026-03-20
severity: medium
components:
  - mika-common/telemetry
  - mika-common/claude
  - mika-common/llm/openai
  - mika-agent/agent
  - mika-agent/server/handlers
tags:
  - langfuse
  - opentelemetry
  - tracing
  - gen_ai
  - span-filtering
  - telemetry
---

# Langfuse Receiving Non-LLM Infrastructure Spans

## Problem

Langfuse was receiving all tracing spans from the application — HTTP request spans from `tower-http::TraceLayer`, internal infrastructure spans, etc. — consuming Langfuse usage quotas and cluttering traces. Meanwhile, actual LLM API calls (`ClaudeClient::send_message()`, `OpenAiCompatibleProvider::send_message()`) emitted only `tracing::info!` log events (not spans), so Langfuse had nothing to classify as "generation" type. Langfuse confirmed the issue via email, noting the account was sending a high share of non-LLM observations.

## Root Cause

Two issues:

1. **No per-layer filtering on the OTel layer.** The `OpenTelemetryLayer` in `logging.rs` received the same global `EnvFilter` as the fmt layers. Every span at `info` level or above was exported to the OTLP endpoint.

2. **LLM calls were events, not spans.** Both providers used `info!()` events with custom field names (`model`, `input_tokens`), not spans with OpenTelemetry `gen_ai.*` semantic convention attributes. Langfuse classifies spans as "generation" only when they have `gen_ai.operation.name` (priority 3) or `gen_ai.request.model` (priority 9 fallback).

## Solution

**Target-based tracing filter** with gen_ai semantic convention spans.

### 1. Per-layer filter on OTel layer (`telemetry.rs`)

Added `filter::Targets` inside `build_otel_layer()` so only spans with `target: "mika::otel"` reach the OTLP exporter:

```rust
let filtered_layer = layer.with_filter(
    filter::Targets::new().with_target("mika::otel", tracing::Level::INFO),
);
```

Return type changed from concrete `OpenTelemetryLayer<S, SdkTracer>` to `impl Layer<Registry> + Send + Sync + use<>` (Rust edition 2024 precise capturing to avoid lifetime capture from `&Settings`).

### 2. LLM call spans with gen_ai attributes (`claude.rs`, `openai.rs`)

Both providers now wrap API calls in `info_span!(target: "mika::otel", "llm_call")` with gen_ai attributes set via `OpenTelemetrySpanExt::set_attribute()`, feature-gated behind `#[cfg(feature = "telemetry")]`:

- `gen_ai.operation.name = "chat"` — Langfuse maps this at priority 3 to GENERATION type
- `gen_ai.provider.name` — `"anthropic"` or `provider_kind.to_string()`
- `gen_ai.request.model`, `gen_ai.request.max_tokens` — set before API call
- `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons` — set after response

The `send_message` method was split into `send_message` (span + telemetry wrapper) and `send_message_inner` (retry logic), with `.instrument(span.clone())` bridging them.

### 3. Parent span tagging (`agent.rs`, `handlers.rs`)

`agent_turn` (conversation, silent, team) and `process_message` spans tagged with `target: "mika::otel"` so Langfuse shows a proper trace tree. Infrastructure spans (`flush_failed_sends`, `compaction`) deliberately NOT tagged.

## Key Decisions

- **Filter applied inside `build_otel_layer()`**, not in `logging.rs` match arms — avoids type-level explosion (see `docs/solutions/architecture-patterns/log-format-selection-tracing-subscriber.md`).
- **`set_attribute()` over tracing field names** — gen_ai attribute keys have dots (`gen_ai.operation.name`) which work as tracing field names but `set_attribute()` is cleaner and naturally feature-gated.
- **No prompt/response content in spans** — only metadata (model, tokens, stop reason). Sensitive data stays out of Langfuse.
- **`use<>` precise capturing** — required in Rust 2024 edition for `impl Trait` return types that take `&Settings` but don't capture the lifetime.

## Prevention

- When adding new spans that should reach Langfuse, always use `target: "mika::otel"`. Without it, the `Targets` filter silently drops the span.
- When adding new LLM providers, follow the `send_message` / `send_message_inner` split pattern and set gen_ai attributes in the wrapper.
- Test both `cargo test` and `cargo test --features telemetry` — the `#[cfg(feature = "telemetry")]` blocks must compile in both modes.
