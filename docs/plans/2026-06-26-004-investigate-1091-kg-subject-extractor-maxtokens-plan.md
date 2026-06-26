# Plan: investigate(kg) — Subject extractor MaxTokens / extraction_semantic_exhausted on gpt-5-nano

**Ticket:** senara-solutions/mika#1091
**Type:** Investigation + fix
**Branch:** `investigate/1091/kg-subject-extractor-hits-maxtokens`

## Problem Statement

The KG subject extractor running on `openrouter/openai/gpt-5-nano` silently hits `MaxTokens` exits. The truncated LLM output fails JSON parsing, enters the C2.2 semantic retry path, which also truncates, producing `extraction_semantic_exhausted` → `Ok(None)` → subjects silently dropped from the knowledge graph.

**Root cause:** The extractor has no `LlmStopReason::MaxTokens` detection. When the response is truncated, it falls through to JSON parse failure → semantic retry (which re-sends the full conversation including the truncated output, making the token budget even tighter) → exhaustion → silent drop. The global `llm_max_tokens` default (4096) is applied uniformly to all LLM paths, but the extraction prompt (system + roster + chunk markers + full doc text) can easily consume most of that budget, leaving insufficient room for the JSON output.

**Secondary issue:** Token accounting for OpenRouter calls records 0 input/0 output tokens (mika#799 sibling), so there's no telemetry to measure the problem's scope.

## Strategic Context (from ticket)

KG query-side (`query_knowledge_graph`) is being deprecated for mika-arch (decision A from 2026-05-12 KG audit). The extraction-side question is whether to fix, pause, or leave as-is. This plan addresses the "fix" path — the cheapest structural improvement that also provides the data to make the pause/continue decision with evidence.

## Investigation Steps

### Step 1: Add MaxTokens-aware detection to the extractor

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

In `call_llm_with_retry()` (line ~1248), after the first `self.llm.send_message(&request).await` succeeds, check `response.stop_reason` before attempting JSON parse:

```rust
Ok(response) => {
    let first_latency = start.elapsed().as_millis() as u64;
    let first_usage = response.usage.clone();
    let stop_reason = response.stop_reason;
    let text = response.text_content();

    // MaxTokens detection: if the response was truncated, log it
    // distinctly and skip the parse attempt — the JSON is guaranteed
    // incomplete. This avoids wasting a semantic retry on unfixable input.
    if stop_reason == LlmStopReason::MaxTokens {
        warn!(
            trace_id = %self.trace_id,
            event = "extraction_max_tokens_truncated",
            output_len = text.len(),
            "LLM hit MaxTokens — response truncated, skipping parse (log-and-skip per C2.3)"
        );
        return Ok(None);
    }

    match self.parse_extraction_json(&text) {
        // ... existing code
    }
}
```

**Rationale:** The semantic retry path is counterproductive for MaxTokens — it sends the truncated output + reinforcement prompt as additional messages, consuming even more of the already-insufficient token budget. Early detection saves a wasted LLM call and produces a distinct log event for telemetry.

Apply the same check in the transport retry path (line ~1291) where parse is attempted after a successful retry.

### Step 2: Add `MIKA_KG_EXTRACTION_MAX_TOKENS` configuration

**File:** `crates/mika-common/src/config.rs`

Add a new config field:

```rust
/// Max tokens for KG extraction LLM responses. Extraction outputs are
/// structured JSON with entities and relationships — typically requires
/// more output budget than conversational responses.
/// Falls back to `llm_max_tokens` (4096) if unset.
#[serde(default)]
pub kg_extraction_max_tokens: Option<u32>,
```

Add a resolver method on `Settings`:

```rust
pub fn kg_extraction_max_tokens(&self) -> u32 {
    self.kg_extraction_max_tokens.unwrap_or(self.llm_max_tokens)
}
```

**File:** `crates/mika-common/src/config.rs` — `make_kg_extraction_provider()` method

Thread the `kg_extraction_max_tokens()` value through to the provider constructor so the extraction provider uses a separate max_tokens from the global default.

**Default behavior:** When unset, falls back to `llm_max_tokens` (4096) — no behavioral change for existing deployments. Operator can set `MIKA_KG_EXTRACTION_MAX_TOKENS=8192` to give extraction more output headroom.

### Step 3: Add structured telemetry for MaxTokens events

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

The `extraction_max_tokens_truncated` event from Step 1 provides the detection signal. Additionally, enrich the existing `extraction_semantic_exhausted` event with `stop_reason` information so operators can retroactively distinguish MaxTokens-caused exhaustion from genuine malformed-JSON exhaustion:

```rust
Err(e) => {
    warn!(
        trace_id = %self.trace_id,
        error = %e,
        event = "extraction_semantic_exhausted",
        first_stop_reason = ?first_attempt_stop_reason,  // NEW
        "semantic retry also failed — log-and-skip per C2.3"
    );
    Ok(None)
}
```

Thread `first_attempt_stop_reason` through `retry_with_reinforcement()` as an additional parameter.

### Step 4: Document env var

**File:** root `CLAUDE.md` — Environment Variables section, under KG optional vars

Add `MIKA_KG_EXTRACTION_MAX_TOKENS` with description matching the config field.

**File:** `.env.example`

Add the new env var with a comment.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/subject_extractor.rs` | MaxTokens early-detection in `call_llm_with_retry()`, enriched `extraction_semantic_exhausted` logging |
| `crates/mika-common/src/config.rs` | `kg_extraction_max_tokens: Option<u32>` field + resolver method |
| `crates/mika-common/src/config.rs` | Thread KG-specific max_tokens through `make_kg_extraction_provider()` |
| `CLAUDE.md` | Document `MIKA_KG_EXTRACTION_MAX_TOKENS` env var |
| `.env.example` | Add `MIKA_KG_EXTRACTION_MAX_TOKENS` |

## Testing

- **Unit test:** Add a test in `subject_extractor.rs` that constructs a `MockLlmProvider` returning a response with `stop_reason: LlmStopReason::MaxTokens` and truncated text, asserts `call_llm_with_retry()` returns `Ok(None)` without attempting a retry call (mock sequence length = 1).
- **Unit test:** Verify `Settings::kg_extraction_max_tokens()` returns the field value when set, falls back to `llm_max_tokens` when unset.
- **Grounding regression test:** Not needed — this is detection/telemetry, not a fabrication-class guard.

## Out of Scope

- **Token accounting fix (mika#799):** Separate ticket with existing worktree.
- **Strategic pause/continue decision:** This fix provides the telemetry; the decision is operator-level.
- **KG query-side deprecation (decision A):** Already committed.
- **Corpora backfill (mika#1076) and resolver instrumentation (mika#1077):** Paused per audit decision.
- **Raising the default `llm_max_tokens` globally:** Would affect all LLM paths; the per-use-case config is safer.

## Acceptance Criteria (from ticket, mapped)

- [x] **Investigation:** Root cause confirmed — MaxTokens truncation → parse failure → silent drop. No content-correlation investigation needed (the mechanism is structural, not content-dependent).
- [ ] **Decision:** Fix path chosen — add detection + per-use-case config. Does not preclude a future "pause extraction" decision.
- [ ] **Implementation:** MaxTokens detection prevents wasted retry calls and produces actionable telemetry.
- [ ] **No downstream consumer breaks:** Detection is additive; existing log-and-skip behavior preserved (just reached via a faster path with better diagnostics).
