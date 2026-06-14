---
title: "fix: Filter non-LLM spans from Langfuse export and add gen_ai semantic conventions"
type: fix
status: completed
date: 2026-03-20
origin: docs/brainstorms/2026-03-20-langfuse-llm-span-filtering-brainstorm.md
---

# Fix Langfuse LLM Span Filtering

## Overview

Non-LLM infrastructure spans (tower-http request spans, etc.) are being exported to Langfuse via OTLP, consuming usage and cluttering traces. Meanwhile, actual LLM calls (`ClaudeClient::send_message()`, `OpenAiClient::send_message()`) emit only `tracing::info!` log events — not spans — so Langfuse has nothing to classify as a "generation". Fix both problems: add gen_ai-attributed spans around LLM calls and filter the OTel layer to only export those spans.

## Problem Statement

1. **LLM calls invisible in Langfuse**: Both providers use `info!()` events (not spans). No `gen_ai.*` attributes. Langfuse cannot classify anything as a "generation".
2. **Infrastructure spans exported**: `tower-http::TraceLayer` HTTP request spans and all `info`-level tracing spans flow to the OTLP exporter unfiltered, consuming Langfuse usage (confirmed by Langfuse email from Lotte).
3. **No per-layer filter**: The OTel layer in `logging.rs` receives the same global `EnvFilter` as fmt layers — no selective export.

## Proposed Solution

**Target-based tracing filter** (see brainstorm: `docs/brainstorms/2026-03-20-langfuse-llm-span-filtering-brainstorm.md`):

1. Assign tracing target `"mika::otel"` to spans that should be exported to Langfuse
2. Apply a `filter::Targets` per-layer filter on the OTel layer: only `mika::otel` spans pass through
3. Create new `llm_call` spans around LLM API calls with `gen_ai.*` attributes via `OpenTelemetrySpanExt::set_attribute()`
4. Tag existing `agent_turn` and `process_message` parent spans with `target: "mika::otel"` for trace tree structure

## Acceptance Criteria

- [x] LLM calls appear as "generation" type in Langfuse with model, token counts, stop reason
- [x] Infrastructure spans (tower-http, etc.) are NOT exported to Langfuse
- [x] `agent_turn` and `process_message` spans appear as parent traces in Langfuse
- [x] Both Anthropic (`ClaudeClient`) and OpenAI-compatible providers are instrumented
- [x] `cargo test` passes (no telemetry feature)
- [x] `cargo test --features telemetry` passes
- [x] `cargo clippy --features telemetry` clean
- [x] Existing `info!` log events preserved for stdout/file logging
- [x] `generate_trace_id()` in `trace.rs` still correctly extracts OTel trace ID

## MVP

### Step 1: Add per-layer filter to OTel layer

**File: `crates/mika-common/src/telemetry.rs`**

Apply `filter::Targets` inside `build_otel_layer()` before returning, so `logging::init()` doesn't need type changes (the learnings doc warns about type explosion in logging.rs's 4-arm match).

```rust
// telemetry.rs — in build_otel_layer()
use tracing_subscriber::filter;

let layer = tracing_opentelemetry::layer().with_tracer(tracer);

// Only export spans from the "mika::otel" target to Langfuse
let filtered_layer = layer.with_filter(
    filter::Targets::new()
        .with_target("mika::otel", tracing::Level::INFO)
);

Some((filtered_layer, guard))
```

This changes the return type from `OpenTelemetryLayer<R, T>` to `Filtered<OpenTelemetryLayer<R, T>, Targets, R>`, but since callers use generic `impl Layer<Registry>`, it's transparent.

**Verify**: `generate_trace_id()` in `trace.rs` still works — it uses `Span::current().context()` from `OpenTelemetrySpanExt`, which should still return the OTel context for spans that pass the filter.

### Step 2: Tag parent spans with `target: "mika::otel"`

**File: `crates/mika-agent/src/agent.rs`**

Tag `agent_turn` spans so they pass the OTel filter and provide trace structure in Langfuse:

```rust
// Conversation mode (line ~693)
let span = info_span!(
    target: "mika::otel",
    "agent_turn",
    agent = %agent_name,
    mode = "conversation",
    trace_id = %trace_id,
    channel = %params.channel_type,
);

// Silent mode (line ~1296)
let silent_span = info_span!(
    target: "mika::otel",
    "agent_turn",
    agent = %...,
    mode = "silent",
    trigger = %trigger_label,
);

// Team agent (line ~1655)
.instrument(tracing::info_span!(target: "mika::otel", "team_agent", agent = %params.agent_name))
```

**File: `crates/mika-agent/src/server/handlers.rs`**

```rust
// process_message span (line ~182)
let span = tracing::info_span!(target: "mika::otel", "process_message", request_id = %request_id);
```

**Do NOT tag**: `flush_failed_sends`, `compaction` — these are infrastructure spans, not user-facing traces.

### Step 3: Add gen_ai spans to ClaudeClient

**File: `crates/mika-common/src/claude.rs`**

Wrap the LLM call in a span with gen_ai attributes. Use `OpenTelemetrySpanExt::set_attribute()` for dotted attribute names, feature-gated behind `#[cfg(feature = "telemetry")]`:

```rust
pub async fn send_message(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
    let span = info_span!(
        target: "mika::otel",
        "llm_call",
        model = %request.model,
        max_tokens = request.max_tokens,
    );

    // Set gen_ai request attributes for Langfuse
    #[cfg(feature = "telemetry")]
    {
        use opentelemetry::KeyValue;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        span.set_attribute(KeyValue::new("gen_ai.operation.name", "chat"));
        span.set_attribute(KeyValue::new("gen_ai.provider.name", "anthropic"));
        span.set_attribute(KeyValue::new("gen_ai.request.model", request.model.to_string()));
        span.set_attribute(KeyValue::new("gen_ai.request.max_tokens", request.max_tokens as i64));
    }

    let response = async {
        info!(model = %request.model, max_tokens = request.max_tokens, "llm_call started");

        // ... existing retry loop ...

        info!(
            model = %request.model,
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            stop_reason = ?response.stop_reason,
            "llm_call completed"
        );
        Ok(response)
    }
    .instrument(span.clone())
    .await?;

    // Set gen_ai response attributes for Langfuse
    #[cfg(feature = "telemetry")]
    {
        use opentelemetry::KeyValue;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        span.set_attribute(KeyValue::new("gen_ai.usage.input_tokens", response.usage.input_tokens as i64));
        span.set_attribute(KeyValue::new("gen_ai.usage.output_tokens", response.usage.output_tokens as i64));
        if let Some(ref reason) = response.stop_reason {
            span.set_attribute(KeyValue::new("gen_ai.response.finish_reasons", format!("{reason:?}")));
        }
    }

    Ok(response)
}
```

**Keep existing `info!` events** — they continue to serve stdout/file logging (unaffected by the OTel filter since they're on the default target).

### Step 4: Add gen_ai spans to OpenAI-compatible provider

**File: `crates/mika-common/src/llm/openai.rs`**

Same pattern as Step 3, but use `self.provider_kind` for `gen_ai.provider.name`:

```rust
span.set_attribute(KeyValue::new("gen_ai.provider.name", self.provider_kind.to_string()));
```

Provider kinds: `"openai"`, `"ollama"`, `"groq"`, `"together"`, `"vllm"`, etc.

### Step 5: Update CLAUDE.md and docs

**File: `CLAUDE.md`** — Update the Observability bullet to mention gen_ai semantic conventions and target-based filtering:

> **Observability:** "Always instrument, optionally export" pattern. LLM call spans use `target: "mika::otel"` with `gen_ai.*` semantic convention attributes for Langfuse generation classification. Per-layer `filter::Targets` on the OTel layer ensures only `mika::otel` spans are exported.

**File: `docs/configuration.md`** — If telemetry env vars are documented there, add a note that only `mika::otel`-targeted spans are exported.

### Step 6: Verify

1. `cargo build --features telemetry` — compiles
2. `cargo build` — compiles (no telemetry, all `#[cfg]` fallbacks work)
3. `cargo test` — passes
4. `cargo test --features telemetry` — passes
5. `cargo clippy --features telemetry` — clean
6. Manual verification: run `mika-spirit` with telemetry enabled pointing at Langfuse, send a message, check Langfuse trace shows:
   - `process_message` trace root
   - `agent_turn` child span
   - `llm_call` generation span(s) with gen_ai attributes
   - No tower-http or infrastructure spans

## Technical Considerations

### Type system impact
Adding `.with_filter()` wraps the layer in `Filtered<L, F, S>`. Apply the filter inside `build_otel_layer()` so `logging::init()` receives the already-filtered layer via its generic parameter — no match arm explosion (per learnings: `docs/solutions/architecture-patterns/log-format-selection-tracing-subscriber.md`).

### Feature gate discipline
All `set_attribute()` calls and `use tracing_opentelemetry::OpenTelemetrySpanExt` imports must be inside `#[cfg(feature = "telemetry")]` blocks. The `info_span!` itself (with `target: "mika::otel"`) is always compiled — the tracing layer simply ignores spans from targets not in the subscriber filter when telemetry is off (they're zero-cost no-ops since there's no OTel layer to receive them).

### Trace ID bridge
`generate_trace_id()` in `trace.rs` calls `Span::current().context()`. Since `agent_turn` has `target: "mika::otel"` and passes the OTel layer filter, the OTel context is created for it. Child `llm_call` spans inherit the OTel context. No change needed to `trace.rs`.

### Span hierarchy in Langfuse
```
process_message (request_id)        ← trace root (server mode)
  └── agent_turn (agent, mode, trace_id, channel)
        ├── llm_call (gen_ai.* attrs)  ← generation #1
        ├── llm_call (gen_ai.* attrs)  ← generation #2 (if tool use loop)
        └── ...
```
CLI mode: `agent_turn` is the trace root (no `process_message`).

## Sources

- **Origin brainstorm:** [docs/brainstorms/2026-03-20-langfuse-llm-span-filtering-brainstorm.md](docs/brainstorms/2026-03-20-langfuse-llm-span-filtering-brainstorm.md) — target-based filter approach, gen_ai attribute set, trace tree decision
- **Learnings — type explosion:** [docs/solutions/architecture-patterns/log-format-selection-tracing-subscriber.md](docs/solutions/architecture-patterns/log-format-selection-tracing-subscriber.md) — apply filter before passing to `init()`
- **Learnings — trace_id correlation:** [docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md](docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md) — verify bridge still works
- **Learnings — OTLP endpoint:** [docs/solutions/integration-issues/otlp-endpoint-path-requirement.md](docs/solutions/integration-issues/otlp-endpoint-path-requirement.md) — full `/v1/traces` path required
- **Langfuse ObservationTypeMapper:** `gen_ai.operation.name = "chat"` → priority 3 GENERATION classification
- **OpenTelemetry gen_ai semantic conventions:** [opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/)
