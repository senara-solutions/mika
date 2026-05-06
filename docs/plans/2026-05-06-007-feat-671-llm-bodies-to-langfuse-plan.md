---
title: "feat: Attach LLM request/response bodies to Langfuse generation spans via OTLP"
type: feat
status: active
date: 2026-05-06
issue: 671
---

# feat: Attach LLM request/response bodies to Langfuse generation spans via OTLP

## Overview

When `MIKA_LOG_LLM_BODIES` is enabled alongside telemetry, attach serialized LLM request and response bodies as `gen_ai.prompt` / `gen_ai.completion` span attributes on existing `llm_call` spans. This sends bodies to Langfuse's Generation input/output fields via the existing OTLP pipeline. Local log file output remains unchanged as the offline fallback.

## Problem Frame

`MIKA_LOG_LLM_BODIES` currently writes full LLM request/response JSON to local log files via the `mika::llm_debug` tracing target. This is useful for local debugging but bodies are disconnected from their traces in Langfuse. Langfuse is purpose-built for this — bodies belong with their generation spans, correlated with token counts, latency, and trace context.

The prior architecture (documented in `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`) explicitly excluded body content from spans: "No prompt/response content in spans — only metadata." Issue #671 reverses that decision, gated behind the existing `log_llm_bodies` toggle so the operator retains control.

## Requirements Trace

- R1. When `log_llm_bodies=true` AND telemetry is enabled, LLM request bodies appear in Langfuse Generation "Input" field
- R2. When `log_llm_bodies=true` AND telemetry is enabled, LLM response bodies appear in Langfuse Generation "Output" field
- R3. Local log file output (`mika::llm_debug` target) continues to work unchanged when telemetry is disabled
- R4. When `log_llm_bodies=false` (default), no body content is attached to spans — preserving the current behavior
- R5. Body content is truncated defensively to prevent runaway payloads (50K char cap, matching `llm_calls.response_text`)
- R6. Both Anthropic and OpenAI-compatible providers attach bodies symmetrically
- R7. Response body serialization reuses the `response_text` format from #653 (text blocks joined, tool call summaries, internal tags stripped) rather than raw API response JSON

## Scope Boundaries

- No new config option — reuses existing `MIKA_LOG_LLM_BODIES` toggle
- No Langfuse native API integration — bodies flow through existing OTLP pipeline
- No changes to the OTLP transport, TracerProvider, or layer filtering
- No changes to `llm_calls` table schema or SQLite storage
- Request body serialization uses provider-specific wire format JSON (already computed for `mika::llm_debug` logging)

### Deferred to Separate Tasks

- Separate `MIKA_TELEMETRY_LLM_BODIES` toggle independent of file logging: future iteration if operators want bodies in Langfuse without local file logging
- Request body size optimization (e.g., omitting system prompt, summarizing tool definitions): future iteration based on Langfuse usage patterns

## Context & Research

### Relevant Code and Patterns

- **Span creation + attribute setting:** `claude.rs:398-436` (Anthropic) and `openai.rs:334-373` (OpenAI-compatible) — `info_span!(target: "mika::otel", "llm_call", ...)` with `#[cfg(feature = "telemetry")]` blocks using `OpenTelemetrySpanExt::set_attribute()`
- **Body logging sites:** `claude.rs:640-680` (Anthropic `send_once`) and `openai.rs:190-232` (OpenAI `send_once`) — gated by `tracing::enabled!(target: "mika::llm_debug", ...)`
- **Response text serialization:** `agent.rs:919-940` — joins text blocks, formats tool calls as `[Tool Call: name(args)]`, strips internal tags, truncates to 50K chars
- **Provider construction:** `llm/mod.rs:329-358` — `create_provider()` takes `ModelSpec` + `max_tokens`, no `Settings` reference
- **Telemetry layer filter:** `telemetry.rs:91` — `filter::Targets::new().with_target("mika::otel", tracing::Level::INFO)` — only `mika::otel` spans reach OTLP
- **Text truncation:** `db::truncate_chars()` (char-count, appends "...") and `text::safe_truncate()` (byte-count, `&str`)

### Institutional Learnings

- `langfuse-non-llm-span-filtering.md`: Prior explicit decision "No prompt/response content in spans — only metadata." This feature reverses that, gated behind `log_llm_bodies`
- `runtime-observability-llm-tool-call-recording.md`: `log_llm_bodies` deliberately uses `mika::llm_debug` (not `mika::otel`) "to avoid sending sensitive content to Langfuse" — the 50KB truncation uses `is_char_boundary()` for safe UTF-8
- `653-llm-call-detail-response-content-linked-tool-calls.md`: Schema v31 `response_text` serialization format is the canonical response body representation — reuse it
- `otlp-endpoint-path-requirement.md`: OTLP exporter silently swallows failures — test with real Langfuse to verify large attributes arrive

### External References

- **OTLP attribute size limits:** OTel spec default is unlimited (no `AttributeValueLengthLimit`). Rust SDK does not auto-truncate. OTLP HTTP has no per-attribute protocol limit; practical limit is transport message size (~4MB for gRPC, server-dependent for HTTP)
- **Langfuse OTLP mapping:** `gen_ai.prompt` → Generation "Input" field; `gen_ai.completion` → Generation "Output" field. Langfuse does NOT support the newer events-based GenAI semconv format (span events `gen_ai.client.inference.operation.details`) — attribute-based format is required
- **Langfuse size limits:** 5MB per API request (Cloud), ~1MB per trace rendering. A 50K-char body (~50KB) is well within all limits
- **Langfuse issue #12657:** Events-based GenAI conventions not yet supported — must use span attributes

## Key Technical Decisions

- **Reuse `log_llm_bodies` toggle (not a new config):** The operator already uses this to opt into body visibility. Adding a separate `MIKA_TELEMETRY_LLM_BODIES` creates unnecessary config surface for the v1 implementation. If orthogonal control is needed later, it's additive.
- **Thread `log_llm_bodies` into providers via struct field:** The `ClaudeClient` and `OpenAiCompatibleProvider` structs need to know whether to attach body attributes. Adding a `log_llm_bodies: bool` field and plumbing it through constructors is the minimal change. The alternative (checking a global/static) fights the existing constructor-injection pattern.
- **Use `gen_ai.prompt` / `gen_ai.completion` attribute names:** These are the GenAI semantic convention attributes Langfuse maps to its Generation UI. Using `langfuse.observation.input` / `langfuse.observation.output` would couple to Langfuse specifically.
- **Serialize request body as JSON string:** The request body is already serialized to JSON for `mika::llm_debug` logging. Reuse that serialization path for the span attribute, applying the 50K char truncation.
- **Serialize response body in `response_text` format:** Rather than raw API JSON (which includes wire-format noise), use the same human-readable format as `llm_calls.response_text` — text blocks joined with newlines, tool calls as `[Tool Call: name(args)]`, internal tags stripped. This requires computing the serialized form in the provider layer, not just in `agent.rs`.
- **Attach at `send_message_with_deadline` level (not `send_once`):** Body attributes should be set alongside existing `gen_ai.*` attributes in the same `#[cfg(feature = "telemetry")]` blocks. The request body is available before the inner call; the response body after. This keeps the body attribute logic co-located with the other generation metadata rather than scattered in `send_once`.
- **50K char truncation cap:** Matches `llm_calls.response_text` column cap. Well within Langfuse's 5MB request limit. Prevents runaway payloads from poisoning the OTLP pipeline.

## Open Questions

### Resolved During Planning

- **OTLP attribute size limits?** No hard limit in OTel spec or OTLP protocol. Rust SDK does not auto-truncate. 50K chars (~50KB) is well within Langfuse's 5MB request limit and ~1MB trace rendering limit. Resolved: OTLP works fine for our payload sizes.
- **`gen_ai.content.prompt` vs `gen_ai.prompt`?** Langfuse maps `gen_ai.prompt` (NOT `gen_ai.content.prompt`) to its Generation Input field. Resolved: use `gen_ai.prompt` and `gen_ai.completion`.
- **Separate toggle vs reuse `log_llm_bodies`?** Reuse existing toggle for v1. A separate `MIKA_TELEMETRY_LLM_BODIES` is additive if needed later.

### Deferred to Implementation

- **Exact `LlmResponse` → response text serialization in provider layer:** The `response_text` construction currently lives in `agent.rs`. Need to determine whether to extract it to a shared function in `mika-common` or duplicate a simplified version in the providers. Implementation will reveal the cleanest factoring.

## Implementation Units

- [x] **Unit 1: Thread `log_llm_bodies` into provider constructors**

**Goal:** Pass the `log_llm_bodies` setting from `Settings` through `create_provider()` into `ClaudeClient` and `OpenAiCompatibleProvider` as a struct field.

**Requirements:** R4 (gating), R6 (both providers)

**Dependencies:** None

**Files:**
- Modify: `crates/mika-common/src/claude.rs` (add `log_llm_bodies: bool` field to `ClaudeClient`, update `new()` signature)
- Modify: `crates/mika-common/src/llm/openai.rs` (add `log_llm_bodies: bool` field to `OpenAiCompatibleProvider`, update `new()` signature)
- Modify: `crates/mika-common/src/llm/anthropic.rs` (update `AnthropicProvider::new()` to accept and forward the flag)
- Modify: `crates/mika-common/src/llm/mod.rs` (update `create_provider()` to accept `log_llm_bodies: bool` and pass it through)
- Modify: `crates/mika-common/src/llm/mock.rs` (update `MockLlmProvider` if needed for `create_provider` signature compatibility)
- Modify: `crates/mika-agent/src/agent.rs` (pass `settings.log_llm_bodies` to `create_provider()`)
- Modify: `crates/mika-cli/src/main.rs` (pass `log_llm_bodies` to `create_provider()` at CLI call sites)
- Test: `crates/mika-common/src/claude.rs` (existing `ClaudeClient::new` tests — update signatures)

**Approach:**
- Add `log_llm_bodies: bool` to `ClaudeClient`, `OpenAiCompatibleProvider`, and `AnthropicProvider` structs
- Thread through `create_provider(spec, max_tokens, log_llm_bodies)` → provider constructors
- Default to `false` in test helpers and dummy providers
- All existing call sites pass the setting from their `Settings` instance

**Patterns to follow:**
- `provider_kind: ProviderKind` field on `OpenAiCompatibleProvider` — same constructor threading pattern

**Test scenarios:**
- Happy path: `ClaudeClient::new()` with `log_llm_bodies=true` stores the flag; with `false` stores false
- Happy path: `create_provider()` accepts the new parameter and constructs providers successfully
- Edge case: existing tests that construct providers directly compile with the updated signature (compilation is the test)

**Verification:**
- `cargo build` succeeds with no signature mismatches
- `cargo test -p mika-common` passes
- `cargo test -p mika-agent` passes (agent.rs call sites updated)

- [x] **Unit 2: Extract response text serialization to shared function**

**Goal:** Move the `response_text` construction logic from `agent.rs` into a shared function in `mika-common` so both the agent loop and the provider telemetry code can use it.

**Requirements:** R7 (reuse response_text format)

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `crates/mika-common/src/llm/mod.rs` (add `pub fn serialize_response_text(content: &[LlmResponseContent]) -> Option<String>`)
- Modify: `crates/mika-agent/src/agent.rs` (replace inline `response_text` construction with call to the shared function)
- Test: `crates/mika-common/src/llm/mod.rs` (unit tests for the shared function)

**Approach:**
- Extract the logic from `agent.rs:919-940` into `mika_common::llm::serialize_response_text(content: &[LlmResponseContent], max_chars: usize) -> Option<String>`
- Uses `strip_internal_tags()` (already in `mika-common::llm`) and `truncate_chars()` — need to either re-export `truncate_chars` from `mika-common` or use `safe_truncate` with a char-count variant
- The function takes a `max_chars` parameter so callers can use 50K for DB and telemetry
- `agent.rs` becomes a one-liner call to the shared function

**Patterns to follow:**
- `strip_internal_tags()` in `mika-common::llm` — same module, shared utility function pattern

**Test scenarios:**
- Happy path: text-only response → joined text with internal tags stripped, truncated to cap
- Happy path: mixed text + tool call response → text and `[Tool Call: name(args)]` summaries joined
- Edge case: empty content → returns `None`
- Edge case: content exceeding 50K chars → truncated with "..." suffix
- Edge case: tool call args exceeding 200 chars → args truncated in summary

**Verification:**
- `agent.rs` `response_text` construction is a single function call
- `cargo test -p mika-common` passes with new unit tests
- `cargo test -p mika-agent` passes (behavior unchanged)

- [x] **Unit 3: Attach request body as `gen_ai.prompt` span attribute**

**Goal:** When `log_llm_bodies` is true and telemetry feature is enabled, serialize the LLM request body and attach it as a `gen_ai.prompt` attribute on the `llm_call` span.

**Requirements:** R1 (request bodies in Langfuse Input), R4 (gating), R5 (truncation), R6 (both providers)

**Dependencies:** Unit 1 (log_llm_bodies field available on providers)

**Files:**
- Modify: `crates/mika-common/src/claude.rs` (add `gen_ai.prompt` attribute in the pre-call `#[cfg(feature = "telemetry")]` block in `send_message_with_deadline`)
- Modify: `crates/mika-common/src/llm/openai.rs` (same pattern in `send_message_with_deadline`)
- Test: `crates/mika-common/src/claude.rs` (unit test verifying request serialization + truncation)

**Approach:**
- In each provider's `send_message_with_deadline`, after the existing `gen_ai.request.*` attributes, add a guarded block:
  ```
  if self.log_llm_bodies {
      // Serialize the provider-specific request to JSON, truncate to 50K chars, set as gen_ai.prompt
  }
  ```
- For Anthropic: serialize `MessagesRequest` to JSON (same serialization as `send_once` body logging)
- For OpenAI: serialize `OpenAiRequest` to JSON (same as `send_once`)
- Apply `truncate_chars()` / equivalent with 50K cap before setting the attribute
- The serialization cost is only paid when `log_llm_bodies=true` (same gating as existing file logging)

**Patterns to follow:**
- Existing `span.set_attribute("gen_ai.request.model", ...)` calls in the same `#[cfg(feature = "telemetry")]` blocks

**Test scenarios:**
- Happy path: with `log_llm_bodies=true` and telemetry feature, request JSON is serialized and set as attribute (verify serialization produces valid JSON)
- Happy path: with `log_llm_bodies=false`, no serialization occurs (verify via absence of attribute — or test that no JSON serialization is called)
- Edge case: request body exceeding 50K chars → truncated with "..." suffix
- Edge case: request serialization failure → no attribute set, no panic (fire-and-forget)

**Verification:**
- `cargo test --features telemetry -p mika-common` passes
- `cargo test -p mika-common` passes (non-telemetry build)
- Body logging to files still works independently

- [x] **Unit 4: Attach response body as `gen_ai.completion` span attribute**

**Goal:** When `log_llm_bodies` is true and telemetry feature is enabled, serialize the LLM response in `response_text` format and attach it as a `gen_ai.completion` attribute on the `llm_call` span.

**Requirements:** R2 (response bodies in Langfuse Output), R4 (gating), R5 (truncation), R6 (both providers), R7 (response_text format)

**Dependencies:** Unit 1 (log_llm_bodies field), Unit 2 (shared serialization function)

**Files:**
- Modify: `crates/mika-common/src/claude.rs` (add `gen_ai.completion` attribute in the post-call `#[cfg(feature = "telemetry")]` block in `send_message_with_deadline`)
- Modify: `crates/mika-common/src/llm/openai.rs` (same pattern in `send_message_with_deadline`)
- Test: `crates/mika-common/src/claude.rs` (unit test verifying response serialization matches `response_text` format)

**Approach:**
- In each provider's `send_message_with_deadline`, after the existing `gen_ai.usage.*` / `gen_ai.response.*` attributes, add:
  ```
  if self.log_llm_bodies {
      // Convert provider-specific response to LlmResponse content, call serialize_response_text(), set as gen_ai.completion
  }
  ```
- For Anthropic: convert `MessagesResponse.content` blocks to `LlmResponseContent` variants, then call `serialize_response_text()`
- For OpenAI: the response is already converted to `LlmResponse` by `send_message_with_deadline` — use `response.content` directly with `serialize_response_text()`
- Note: In `claude.rs`, the response is `MessagesResponse` (Anthropic-specific), not `LlmResponse`. The conversion to `LlmResponse` happens in `anthropic.rs`. The simplest approach is to serialize the Anthropic response to a similar format: iterate `ContentBlock` variants (Text → text, ToolUse → `[Tool Call: ...]`), strip tags, truncate. Alternatively, move the attribute setting into `anthropic.rs` after the conversion.

**Patterns to follow:**
- `serialize_response_text()` from Unit 2
- Existing `gen_ai.response.finish_reasons` attribute setting in the same blocks

**Test scenarios:**
- Happy path: text-only response → stripped, truncated text set as `gen_ai.completion` attribute
- Happy path: mixed text + tool call response → text with `[Tool Call: ...]` summaries
- Happy path: with `log_llm_bodies=false`, no response serialization occurs
- Edge case: empty response content → no attribute set (matches `response_text = None` behavior)
- Edge case: response exceeding 50K chars → truncated
- Integration: Anthropic response → `gen_ai.completion` attribute matches the `response_text` that would be stored in `llm_calls` table for the same response

**Verification:**
- `cargo test --features telemetry -p mika-common` passes
- `cargo test -p mika-common` passes (non-telemetry build)
- Response format in Langfuse matches what the dashboard shows from `llm_calls.response_text`

- [x] **Unit 5: Update documentation and solution doc**

**Goal:** Document the reversed architectural decision and update relevant docs.

**Requirements:** R3 (local logging unchanged), R4 (gating documented)

**Dependencies:** Units 3, 4

**Files:**
- Modify: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md` (update the "No prompt/response content in spans" decision with a note that #671 reversed this, gated behind `log_llm_bodies`)
- Modify: `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` (note that `log_llm_bodies` now also drives OTLP body attributes when telemetry is enabled)

**Approach:**
- Add a note to each doc referencing #671 and the gating behavior
- Keep the existing content as historical context

**Test expectation: none — documentation only**

**Verification:**
- Docs accurately describe the new dual behavior of `log_llm_bodies`

## System-Wide Impact

- **Interaction graph:** The change is confined to the `llm_call` span attribute setting in two provider implementations (`claude.rs`, `openai.rs`). No callbacks, middleware, or observers are affected. The OTLP export pipeline (`telemetry.rs`) is unchanged — it exports whatever attributes are on `mika::otel` spans.
- **Error propagation:** Body serialization or attribute setting failures must not crash the agent loop. Use the existing fire-and-forget pattern — if JSON serialization fails (`serde_json::to_string` returns `Err`), skip the attribute silently. `set_attribute()` itself does not return errors.
- **State lifecycle risks:** None. Span attributes are ephemeral (exported via OTLP, then discarded). No persistent state changes.
- **API surface parity:** Both Anthropic and OpenAI-compatible providers must implement the same body attachment behavior. The `LlmProvider` trait is unchanged — body attachment is an internal provider concern, not a trait contract.
- **Unchanged invariants:** The `mika::llm_debug` tracing target and local file logging behavior is completely unchanged. The `llm_calls` table storage is unchanged. The OTLP `filter::Targets` filter is unchanged. The `telemetry` feature gate is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Large body attributes increase OTLP payload size, potentially causing export failures | 50K char truncation cap. Langfuse accepts up to 5MB per request; 50K chars (~50KB) is 1% of that. OTLP exporter silently drops failed batches — no agent impact. |
| Sensitive data (API keys in system prompt, user PII) reaches Langfuse | This is an operator opt-in (`log_llm_bodies=true`). The same data already flows to local log files when enabled. Operators who enable it accept the data exposure. |
| Serialization cost on hot path | Gated behind `if self.log_llm_bodies` — zero cost when disabled (default). When enabled, JSON serialization is ~1ms for a 50K request, negligible compared to the LLM API call latency. |
| `ClaudeClient` response is `MessagesResponse` (Anthropic-specific), not `LlmResponse` | May need a lightweight conversion or dedicated serialization for Anthropic `ContentBlock` types. Implementation will determine the cleanest factoring. |

## Sources & References

- Related issue: #671
- Related issue: #653 (LLM call detail — response_text column)
- Predecessor doc: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- Pattern doc: `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
- Langfuse OTLP docs: https://langfuse.com/integrations/native/opentelemetry
- Langfuse GenAI events issue: https://github.com/langfuse/langfuse/issues/12657
- OTel attribute limits spec: https://opentelemetry.io/docs/specs/otel/common/
