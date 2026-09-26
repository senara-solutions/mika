# Langfuse LLM Span Filtering

**Date:** 2026-03-20
**Status:** Accepted

## What We're Building

Fix Langfuse observability so that only LLM-relevant spans are exported, and those spans carry proper `gen_ai.*` semantic convention attributes so Langfuse classifies them as "generations" (not plain spans).

**The problem today:**
- `ClaudeClient::send_message()` and `OpenAiClient::send_message()` emit `tracing::info!` log *events* — not dedicated spans. No `gen_ai.*` attributes. Langfuse has nothing to classify as a "generation".
- `tower-http::TraceLayer` generates HTTP request spans for every inbound request. These are the non-LLM infrastructure spans that Langfuse flagged (email from Lotte) — they consume usage and clutter traces.
- The OTel layer has no per-layer filter — every `tracing` span at `info` level or above flows to the OTLP exporter.

**What Langfuse needs to classify a span as a "generation":**
- `gen_ai.operation.name = "chat"` (priority 3 in Langfuse's mapper), OR
- Any `gen_ai.request.model` attribute present (priority 9 fallback)
- Without these, spans default to plain "SPAN" type

## Why This Approach

**Target-based tracing filter** (Approach A) — opted in after evaluating three approaches:

1. **Target-based tracing filter (chosen):** Assign a dedicated tracing target (`mika::otel`) to spans that should be exported. Add a per-layer filter on the OTel layer so only `mika::otel` spans flow into OpenTelemetry. Most idiomatic Rust, filtering happens at subscriber level (before OTel span creation), minimal custom code.

2. **Custom SpanProcessor filter (rejected):** Attribute-based filtering at OTel SDK level. More custom code, and spans are still created in OTel before being dropped — wasteful.

3. **Dedicated OTel-only spans (rejected):** Bypass the tracing bridge entirely. Maximum control but adds complexity with two instrumentation systems and loses tracing context propagation.

## Key Decisions

1. **Scope: gen_ai spans + filtering** — both problems fixed together. Not just filtering (which would leave LLM calls invisible) and not just gen_ai spans (which would still waste network/usage on infrastructure spans).

2. **Target-based filter** — spans opt-in to export via `target: "mika::otel"`. The OTel layer gets `.with_filter(filter::Targets::new().with_target("mika::otel", Level::INFO))`.

3. **Standard metadata, no prompt content** — gen_ai spans carry: `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.request.max_tokens`, `gen_ai.request.temperature`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`. No `gen_ai.input.messages` or `gen_ai.output.messages` (sensitive, large).

4. **Keep trace structure** — export `agent_turn` and `process_message` parent spans (tagged with `mika::otel` target) so Langfuse shows a proper trace tree: trace → generation(s).

5. **Both providers** — instrument `ClaudeClient` (Anthropic) and `OpenAiClient` (OpenAI-compatible: OpenAI, Ollama, Groq, Together, vLLM).

## What Changes

### Files to modify:
- `crates/mika-common/src/telemetry.rs` — add per-layer filter to OTel layer
- `crates/mika-common/src/logging.rs` — apply the filtered OTel layer
- `crates/mika-common/src/claude.rs` — wrap `send_message()` in gen_ai span
- `crates/mika-common/src/llm/openai.rs` — wrap `send_message()` in gen_ai span
- `crates/mika-agent/src/agent.rs` — tag `agent_turn` spans with `target: "mika::otel"`
- `crates/mika-agent/src/server/handlers.rs` — tag `process_message` span with `target: "mika::otel"`

### gen_ai attributes per LLM call span:
| Attribute | Source |
|-----------|--------|
| `gen_ai.operation.name` | `"chat"` |
| `gen_ai.provider.name` | `"anthropic"` / `"openai"` / `"ollama"` etc. |
| `gen_ai.request.model` | model name from config |
| `gen_ai.request.max_tokens` | max_tokens parameter |
| `gen_ai.request.temperature` | temperature if set |
| `gen_ai.usage.input_tokens` | from API response |
| `gen_ai.usage.output_tokens` | from API response |
| `gen_ai.response.finish_reasons` | stop_reason from response |

### Filtering mechanism:
```rust
// In logging.rs, when composing the subscriber:
let otel_layer = otel_layer.map(|layer| {
    layer.with_filter(
        filter::Targets::new()
            .with_target("mika::otel", Level::INFO)
    )
});
```

### LLM call span pattern:
```rust
// In claude.rs send_message():
let span = info_span!(
    target: "mika::otel",
    "chat",  // span name = "{operation} {model}" convention
    "gen_ai.operation.name" = "chat",
    "gen_ai.provider.name" = "anthropic",
    "gen_ai.request.model" = %model,
    "gen_ai.request.max_tokens" = max_tokens,
);
// After response:
span.record("gen_ai.usage.input_tokens", input_tokens);
span.record("gen_ai.usage.output_tokens", output_tokens);
span.record("gen_ai.response.finish_reasons", stop_reason);
```

## Open Questions

None — all key decisions resolved during brainstorm.
