# Plan: fix(kg) — Subject extractor log-and-skip on MaxTokens truncation (gpt-5-nano)

**Ticket:** senara-solutions/mika#1091
**Type:** fix
**Branch:** `investigate/1091/kg-subject-extractor-hits-maxtokens`
**Depth:** Lightweight (single-file behavior change + one threaded param + unit tests)

---

## Problem Frame

The KG subject extractor running on `openrouter/openai/gpt-5-nano` silently hits `MaxTokens` exits. The truncated LLM output fails JSON parsing, falls through the C2.2 **semantic retry** path — which re-sends the full conversation *plus* the truncated output, making the token budget even tighter, so it truncates again — and finally logs `extraction_semantic_exhausted` → `Ok(None)` → subjects silently dropped from the knowledge graph.

**Root cause (verified by reading code on this branch):** `call_llm_with_retry()` has **no `LlmStopReason::MaxTokens` detection**. A truncated response returns `Ok(response)` (not `Err`), so the truncation is invisible to the retry taxonomy and is mishandled as a malformed-JSON semantic failure.

**Why this is more than a one-off drop:** On the `Ok(None)` path, `extract_document` returns `Ok(ExtractionStats::default())` at `crates/mika-agent/src/kg/subject_extractor.rs:725-731` — *before* `write_extraction_results` writes the `kg_extractions` idempotency marker (step 7, ~line 789). So a truncating doc **never gets a marker, stays pending, and is re-attempted every 30-minute tick — burning 2 doomed LLM calls per cycle, indefinitely.** The failure is silent (no distinct event) and unbounded in cost.

---

## Scope

**In scope (Vincent's direct dispatch, deliberately narrowed):** "Implement the log-and-skip pattern at the subject extractor on MaxTokens hits per the issue body." Concretely — detect the `MaxTokens` stop-reason, skip *immediately* (no doomed semantic retry), and emit a distinct, greppable telemetry event so the previously-silent failure mode becomes observable.

This is the cheapest structural improvement that also produces the data to make the strategic fix-vs-pause-vs-leave decision later. It does **not** make that decision.

### What this fix does and does NOT change

- **Improves:** the truncation path is now *fast* (one call, not two), *observable* (distinct WARN event), and *budget-cheaper* (eliminates the guaranteed-doomed semantic retry — halves per-cycle cost for truncating docs).
- **Preserves intentionally:** the doc still stays **pending** (no marker newly written). Truncation can be transient — the doc text, the roster-injected prompt section, or the model can change between ticks — so permanently marking the doc "done/skipped" would lose a future-recoverable extraction. Eliminating the perpetual re-attempt is the *strategic* decision (pause/fix/leave), which is out of scope here per the issue body's own "Out of scope" section.

---

## Evidence (code references on this branch)

| Claim | Reference |
|-------|-----------|
| `LlmResponse` carries `stop_reason: LlmStopReason` | `crates/mika-common/src/llm/types.rs:102` |
| `LlmStopReason::MaxTokens` variant exists | `crates/mika-common/src/llm/types.rs:154` |
| OpenAI/OpenRouter map `finish_reason: "length"` → `LlmStopReason::MaxTokens` | `crates/mika-common/src/llm/openai.rs:750` |
| OpenRouter uses the OpenAI-compatible provider path | gpt-5-nano truncation therefore surfaces as `MaxTokens` |
| `Ok(None)` path returns before the idempotency-marker write | `crates/mika-agent/src/kg/subject_extractor.rs:725-731` vs step 7 `~789` |
| Attempt-1 success branch (parse site 1) | `subject_extractor.rs` `call_llm_with_retry` `~1248` |
| Transport-retry success branch (parse site 2) | `subject_extractor.rs` `~1286` |
| Semantic-exhausted WARN (to be enriched) | `subject_extractor.rs` `retry_with_reinforcement` `~1388` |

---

## Key Technical Decisions

**KTD-1: Detect at the response boundary, not the parse boundary.** Check `response.stop_reason == LlmStopReason::MaxTokens` immediately after each successful `send_message`, before `parse_extraction_json`. A truncated response is *guaranteed* to be incomplete JSON; attempting to parse it and then semantically retry is wasted work. Early detection is both correct (distinct cause) and cheaper (no retry).

**KTD-2: Skip both retry arms.** MaxTokens detection returns `Ok(None)` directly — it does NOT enter `retry_with_reinforcement` (semantic) nor the transport-retry loop. Re-sending a longer conversation against the same `max_tokens` budget would truncate again with certainty.

**KTD-3: Distinct event name for observability.** Use `extraction_max_tokens_truncated` (WARN) rather than reusing `extraction_semantic_exhausted`. This makes the failure class greppable and separates "model ran out of output budget" from "model emitted malformed JSON" — two different remediations (raise budget / switch model vs. prompt-shape fix).

**KTD-4: Enrich the exhaustion event too.** Even with KTD-1, a *non-truncated* first attempt can still semantically fail and the retry can truncate. Thread the first-attempt `stop_reason` into `retry_with_reinforcement` and log it on `extraction_semantic_exhausted` so operators can retroactively attribute exhaustion to truncation vs. malformed JSON.

**KTD-5: Preserve stay-pending semantics.** Do not write an idempotency marker on the MaxTokens path. (See Scope § "What this fix does and does NOT change.")

---

## Implementation Units

### U1. MaxTokens early-detection in `call_llm_with_retry()`

**Goal:** Detect a truncated (`MaxTokens`) LLM response at both parse sites and log-and-skip immediately, bypassing the doomed semantic/transport retries.

**Requirements:** Ticket AC "Implementation: MaxTokens detection prevents wasted retry calls and produces actionable telemetry."

**Dependencies:** none.

**Files:**
- `crates/mika-agent/src/kg/subject_extractor.rs` (modify `call_llm_with_retry`)

**Approach:**
- Parse site 1 — the attempt-1 `Ok(response)` branch (`~1248`): after capturing `usage`/`latency`, read `response.stop_reason`. If `== LlmStopReason::MaxTokens`, emit the distinct WARN (`extraction_max_tokens_truncated`) with `trace_id` and `output_len` (= `text.len()`), and `return Ok(None)` before `parse_extraction_json`.
- Parse site 2 — the transport-retry `Ok(response)` branch (`~1286`): apply the identical check before its parse attempt.
- `LlmStopReason` is already reachable via the existing `mika_common::llm` import surface; confirm the import line includes it (add if absent — a one-symbol import, not a new dependency).
- Do NOT alter the `Err(...)` arms (transport/config) — they already log-and-skip correctly.

**Patterns to follow:** Mirror the existing `warn!(trace_id = %self.trace_id, event = "...", ...)` structured-log shape already used for `extraction_transport_exhausted` / `extraction_config_error` in the same function. Return type stays `Result<Option<ExtractionResult>>`; `Ok(None)` is the established log-and-skip signal consumed by `extract_document` at `:725-731`.

**Test scenarios:** see U3.

**Verification:** A unit test proves that a `MaxTokens` response yields `Ok(None)` after exactly one `send_message` (no retry consumed). `cargo build -p mika-agent` and `cargo clippy -p mika-agent` clean.

---

### U2. Enrich `extraction_semantic_exhausted` with first-attempt stop-reason

**Goal:** When semantic retry still fails, record whether the *first* attempt was itself truncated, so exhaustion can be attributed to MaxTokens vs. genuine malformed JSON.

**Requirements:** Ticket AC "produces actionable telemetry"; supports the deferred strategic decision with data.

**Dependencies:** U1 (same function/region; land together to avoid churn).

**Files:**
- `crates/mika-agent/src/kg/subject_extractor.rs` (modify `retry_with_reinforcement` signature + its `extraction_semantic_exhausted` WARN)

**Approach:**
- Add a parameter `first_attempt_stop_reason: LlmStopReason` to `retry_with_reinforcement`.
- Thread it from both call sites that invoke `retry_with_reinforcement` (attempt-1 parse-failure path and transport-retry parse-failure path), passing the `stop_reason` captured from the corresponding successful response.
- Add `first_stop_reason = ?first_attempt_stop_reason` as a field on the existing `extraction_semantic_exhausted` WARN at `~1388`.
- Note: with U1 in place, a `MaxTokens` first attempt no longer *reaches* the semantic retry — so in practice `first_stop_reason` here will usually be `EndTurn`. The field still earns its place: it confirms (rather than assumes) the first attempt was non-truncated, and it remains correct if U1's call-ordering is ever refactored.

**Patterns to follow:** existing `?`-debug field rendering on structured `warn!` calls in this module (e.g. `chunk_indices = ?entity.chunk_indices`).

**Test scenarios:** see U3.

**Verification:** `cargo build` / `cargo clippy` clean; the enriched field appears in the test's captured log (or is asserted via the function's return-path test).

---

### U3. Unit tests for MaxTokens detection

**Goal:** Lock in the log-and-skip-on-truncation behavior and the no-wasted-retry guarantee.

**Requirements:** Testing contract from the dispatch.

**Dependencies:** U1, U2.

**Files:**
- `crates/mika-agent/src/kg/subject_extractor.rs` (`#[cfg(test)] mod tests` — add cases)

**Approach / test scenarios:**
1. **MaxTokens → single call, no retry (happy path of the fix).** Construct a `MockLlmProvider` whose response sequence is length 1, returning `stop_reason: LlmStopReason::MaxTokens` with truncated/partial-JSON text. Drive `call_llm_with_retry`. Assert: returns `Ok(None)`; the mock recorded exactly **one** `send_message` call (the doomed semantic retry was NOT consumed). This is the core regression guard — pre-fix, this path consumed 2 calls and ended in `extraction_semantic_exhausted`.
2. **Non-truncated malformed JSON still retries (no regression to existing semantic path).** Mock sequence length 2: first response `stop_reason: EndTurn` with unparseable text, second response `EndTurn` with valid JSON. Assert: returns `Ok(Some(_))` with the parsed output, and the mock recorded **two** calls. Confirms U1's early-return is gated strictly on `MaxTokens` and does not short-circuit legitimate semantic recovery.
3. **(If cheaply expressible with the mock) MaxTokens in the transport-retry branch.** If the mock harness can exercise parse site 2, assert the same `Ok(None)` + single-effective-parse behavior there. If the harness cannot reach that branch without a transport-error fixture, note it as covered-by-inspection rather than forcing a brittle test.

**Patterns to follow:** Existing `MockLlmProvider` usage in this crate (`mika_common::llm::mock`, sequence-based) and the inline `#[cfg(test)] mod tests` convention. Check whether `subject_extractor.rs` already has mock-based tests to mirror; if extraction tests currently live under `tests/eval/`, follow the closest existing pattern for constructing a `SubjectExtractor` with a mock provider.

**Test expectation:** behavioral — scenarios 1 and 2 are required; scenario 3 is best-effort.

**Verification:** `cargo test -p mika-agent` passes including the new cases.

---

## Out of Scope / Deferred to Follow-Up Work

- **`MIKA_KG_EXTRACTION_MAX_TOKENS` config knob (raise extraction output budget).** This is option **1a** ("raise the extractor's max-tokens") from the issue body — a *different* fix direction than log-and-skip, explicitly enumerated as separate. It would give extraction more output headroom (and could be combined with a per-use-case provider). **Deliberately deferred:** the dispatch scoped to log-and-skip, and whether to spend more budget on extraction (vs. switch model, vs. chunk inputs, vs. pause the path entirely during the `query_knowledge_graph` deprecation window) is the strategic decision that belongs to the operator. The telemetry shipped here (`extraction_max_tokens_truncated` event) is precisely the data needed to make that call. File as a follow-up ticket if/when the strategic direction is chosen.
- **Strategic pause/fix/leave decision** for KG extraction during the deprecation window (issue scope item 2) — operator-level; out of scope per the issue's own "Out of scope."
- **OpenRouter token accounting** recording 0 input/0 output tokens (issue scope item 3) — sibling of mika#799, which has its own worktree.
- **KG query-side deprecation (decision A)** — already committed in the 2026-05-12 brainstorm.
- **Corpora backfill (mika#1076), resolver `no_match` instrumentation (mika#1077)** — paused per audit decision A.
- **Raising the global `llm_max_tokens` default** — would affect every LLM path; a per-use-case lever (the deferred config knob above) is the safer shape if budget is ever the chosen fix.

---

## Acceptance Criteria (from ticket, mapped)

- [x] **Investigation — correlate MaxTokens exits with content vs. distributed.** Resolved structurally: the mechanism is **not content-dependent in a way that changes the fix.** Any doc whose (system + roster + chunk-marked full text) prompt leaves gpt-5-nano insufficient output budget truncates; the extractor's response to truncation is what's broken, regardless of which docs trip it. Per-doc content correlation would refine the *strategic* fix choice (raise budget vs. chunk vs. switch model), which is deferred.
- [x] **Decision — fix path chosen:** log-and-skip cleanly + observable telemetry (this plan). Does not preclude a later pause/raise-budget decision.
- [ ] **Implementation:** U1 (detection + immediate skip) + U2 (enriched exhaustion telemetry). *(satisfied on merge)*
- [ ] **No downstream consumer breaks:** detection is additive; the existing `Ok(None)` log-and-skip contract consumed by `extract_document` is unchanged — the truncation case just reaches it via a faster, observable path. Stay-pending semantics preserved. *(verified by U3 scenario 2 + review)*
