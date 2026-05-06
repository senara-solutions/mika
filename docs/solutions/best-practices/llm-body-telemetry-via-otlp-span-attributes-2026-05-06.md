---
title: "LLM body telemetry via OTLP span attributes for Langfuse"
date: 2026-05-06
category: best-practices
module: mika-common/llm
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding LLM request/response body content to telemetry spans
  - Extending the gen_ai.* semantic convention attributes on llm_call spans
  - Choosing between OTLP span attributes vs Langfuse native API for large payloads
tags:
  - telemetry
  - langfuse
  - otlp
  - opentelemetry
  - gen-ai
  - span-attributes
  - llm-bodies
  - observability
---

# LLM Body Telemetry via OTLP Span Attributes for Langfuse

## Context

`MIKA_LOG_LLM_BODIES` originally wrote full LLM request/response JSON to local log files via the `mika::llm_debug` tracing target. Bodies were disconnected from their Langfuse traces. Issue #671 evaluated whether to send bodies via the existing generic OTLP pipeline or via Langfuse's native API.

The prior architecture (documented in `langfuse-non-llm-span-filtering.md`) explicitly excluded body content from spans: "No prompt/response content in spans -- only metadata." This decision was reversed in #671, gated behind the existing `log_llm_bodies` toggle.

## Guidance

### Use `gen_ai.prompt` and `gen_ai.completion` span attributes

Langfuse maps these specific attribute names to its Generation "Input" and "Output" fields:

- `gen_ai.prompt` -- request body (JSON-serialized wire format)
- `gen_ai.completion` -- response body (human-readable `serialize_response_text` format)

Do NOT use `gen_ai.content.prompt` / `gen_ai.content.completion` (Langfuse doesn't map these). Do NOT use the newer events-based GenAI semconv format (`gen_ai.client.inference.operation.details`) -- Langfuse does not support it (langfuse/langfuse#12657).

### Thread config through provider constructors, not globals

The `log_llm_bodies: bool` field is added to `ClaudeClient` and `OpenAiCompatibleProvider` structs, threaded through `create_provider()`. This follows the existing constructor-injection pattern (like `provider_kind`). Avoid static/global config checks in the provider layer.

### Reuse `serialize_response_text()` for response bodies

`mika_common::llm::serialize_response_text()` is the canonical serialization for LLM response content. It produces the same format used by `llm_calls.response_text` (schema v31): text blocks joined with newlines, tool calls as `[Tool Call: name(args)]`, internal tags stripped, truncated to `MAX_RESPONSE_TEXT_CHARS` (50K).

### Apply defensive truncation (50K chars)

OTLP has no hard per-attribute size limit. Langfuse accepts up to 5MB per request. But apply `truncate_chars()` at 50K chars to prevent runaway payloads from poisoning the pipeline. This matches the `llm_calls.response_text` column cap.

### Feature-gate telemetry code behind `#[cfg(feature = "telemetry")]`

Body attribute attachment lives inside existing `#[cfg(feature = "telemetry")]` blocks alongside `gen_ai.request.model` etc. Use `#[cfg_attr(not(feature = "telemetry"), allow(dead_code))]` on the `log_llm_bodies` struct field to suppress dead-code warnings in the default (non-telemetry) build.

## Why This Matters

Bodies in Langfuse, correlated with token counts, latency, and trace context, are far more useful than bodies scattered in local log files. The OTLP approach keeps the architecture clean (no Langfuse-specific SDK coupling) while the 50K char cap and `log_llm_bodies` gating ensure the feature is safe for production.

## When to Apply

- When adding new telemetry data to LLM call spans
- When considering Langfuse native API vs generic OTLP for new attributes
- When adding new providers that need body telemetry parity

## Examples

Request body attachment (Anthropic provider, `claude.rs`):

```rust
#[cfg(feature = "telemetry")]
{
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    // ... existing gen_ai.request.* attributes ...

    if self.log_llm_bodies
        && let Ok(body_json) = serde_json::to_string(request)
    {
        span.set_attribute(
            "gen_ai.prompt",
            crate::llm::truncate_chars(&body_json, crate::llm::MAX_RESPONSE_TEXT_CHARS),
        );
    }
}
```

Response body attachment (OpenAI-compatible provider, `openai.rs`):

```rust
#[cfg(feature = "telemetry")]
{
    // ... existing gen_ai.usage.* attributes ...

    if self.log_llm_bodies
        && let Some(text) = super::serialize_response_text(
            &response.content,
            super::MAX_RESPONSE_TEXT_CHARS,
        )
    {
        span.set_attribute("gen_ai.completion", text);
    }
}
```

## Related

- Issue: #671
- Predecessor: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- Pattern: `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
- Schema v31: `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md`
- Langfuse OTLP docs: https://langfuse.com/integrations/native/opentelemetry
