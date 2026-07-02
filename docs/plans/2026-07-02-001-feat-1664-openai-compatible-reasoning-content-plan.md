# Plan: Surface OpenAI-compatible `reasoning_content` from Z.AI / reasoning-mode responses (mika#1664)

**Ticket:** mika issue#1664 — `feat(llm): surface OpenAI-compatible reasoning_content field from Z.AI / reasoning-mode responses`
**Labels:** `enhancement`, `p2-normal`, `agent-core`, `ready`
**Type:** issue (enhancement)
**Priority tier:** Tier 4 — *Features* (product capability: reasoning-mode model adoption beyond Anthropic). Carries a Tier-2 flavour (*slows the loop*): today the unparsed field makes GLM-5.2's legitimate reasoning show up as a bogus `EmptyResponse` calibration failure, a misleading signal — but the calibration-signal half (AC4) already shipped separately, so what remains here is the feature half.

---

## Problem

Z.AI's GLM-5.2 (and other reasoning-mode models on OpenAI-compatible APIs) return their internal chain-of-thought in a separate `message.reasoning_content` JSON field, distinct from `message.content`. `OpenAiCompatibleProvider` (`crates/mika-common/src/llm/openai.rs`) parses only `message.content`. When the model spends its entire output-token budget on reasoning, `message.content` is empty (`finish_reason: "length"`), the visible response is blank, and the reasoning trace is silently discarded — never surfaced, never persisted.

Hard evidence (2026-06-30 06:20 UTC, direct Z.AI call with the calibration `refusal_regression` fixture prompt): `finish_reason: "length"`, `content_length: 0`, `reasoning_length: 4507`, `completion_tokens: 1000` (all `reasoning_tokens`). The model was processing the task correctly (`"Let me analyze this task carefully. 1. Remove stale build artifacts…"`) — it just never reached the visible-response phase before the cap.

## Grounded state analysis — what already exists vs. what remains

Investigation of the *current* code (not the ticket's as-of-filing assumptions) shows most of the ticket has already landed via adjacent work. The plan scopes only the genuine gap and does not re-plan done work.

| AC | Claim | Actual current state | Remaining work |
|----|-------|----------------------|----------------|
| **AC1** | `OpenAiResponse` parses `message.reasoning_content` as `Option<String>` | **NOT done.** `grep reasoning_content crates/` returns zero hits. `from_openai_response` (`openai.rs:661`) extracts reasoning *only* from `<think>…</think>` blocks inside `content` (`extract_think_block`, `openai.rs:769-788`); the separate JSON field is never deserialized. | **This is the real work.** |
| **AC2** | `LlmResponse` exposes the reasoning | **Field already exists.** `LlmResponse.reasoning: Option<String>` (`types.rs:101`), populated today by the `<think>` path and consumed at `agent_loop/mod.rs:854`. No named accessor method (`reasoning()`) exists — the `pub` field is read directly. | Field is sufficient; add a thin `reasoning()` accessor + doc for API-surface clarity (AC2 wording: "via a method"). Small. |
| **AC3** | `llm_calls.reasoning` populated when `reasoning_content` present | **Storage path already wired and provider-agnostic.** `agent_loop/mod.rs:854-859` reads `resp.reasoning`, truncates to `MAX_RESPONSE_TEXT_CHARS`, and passes it as the `reasoning_text` arg to `save_llm_call` (`async_db.rs:2284`, column added schema v31). Works for *any* provider that populates `LlmResponse.reasoning`. | **Free once AC1 lands** — AC1 populating `resp.reasoning` automatically flows to the column. Needs a regression test proving the round-trip, no new plumbing. |
| **AC4** | Calibration `classify_failure` distinguishes `ReasoningBudgetExhausted` from `EmptyResponse` | **Already fully implemented** via sibling **mika#1665**. `FailureClass::ReasoningBudgetExhausted` (`calibration/failure.rs:28`), `classify_failure(error, response_text, output_tokens, finish_reason_is_length)` (`failure.rs:79`), and the `empty_response_result` helper (`calibration/roles/mod.rs:37-66`) all exist with tests (`failure.rs:188-220`). | **None.** Verify-only: confirm no regression, cite the existing coverage in the PR. Do not touch. |

**Synthesis:** the one substantive change is AC1 — teach the OpenAI-compatible response deserializer about `reasoning_content` and route it into the already-existing `LlmResponse.reasoning` → `llm_calls.reasoning` pipeline. AC2/AC3 are a small accessor + tests riding on AC1; AC4 is done. This is a ~1-file change plus tests, not a subsystem.

## Requirements

### R1 — Deserialize `message.reasoning_content` (AC1)

In `crates/mika-common/src/llm/openai.rs`, add a response-only `reasoning_content` field to the message shape used for **deserialization**:

- Add `reasoning_content: Option<String>` to `OpenAiMessage` (`openai.rs:26-35`). Because `OpenAiMessage` is shared between request serialization and response deserialization, annotate the new field with **both** `#[serde(default)]` (tolerate absence on every provider that never emits it) and `#[serde(skip_serializing_if = "Option::is_none")]` (never send it back in a request — it is a response-only field; sending it upstream is at best ignored, at worst rejected). This preserves exact request-wire compatibility — verify via the existing `test_openai_request_serialization` (`openai.rs:1217`) and add an assertion that a request body never contains `reasoning_content`.
  - *Design note / open decision for the implementer:* if adding the field to the shared `OpenAiMessage` proves awkward (e.g. it widens the request type in a way the reviewer dislikes), the equally-valid alternative is a response-only struct — deserialize choices with a dedicated `OpenAiResponseMessage` that carries `reasoning_content`, leaving the request-side `OpenAiMessage` untouched. Prefer the single-field-on-shared-struct approach for minimal churn unless the skip-serializing guard is judged insufficient; record the choice in the PR body.

### R2 — Route `reasoning_content` into `LlmResponse.reasoning`, with precedence (AC1 → AC2/AC3)

In `from_openai_response` (`openai.rs:661-788`), after extracting text/tool-calls and running the existing `<think>` extraction:

- If `choice.message.reasoning_content` is `Some(non-empty)`, set `LlmResponse.reasoning` to it.
- **Precedence, explicitly:** the dedicated `reasoning_content` field is authoritative when present. The existing `<think>`-block extraction remains the fallback for providers that inline reasoning in `content` (DeepSeek-R1, MiniMax) and must be preserved unchanged. Define the rule precisely and test both orderings:
  - `reasoning_content` present → use it; do **not** also strip `<think>` (a reasoning-mode provider that returns the structured field will not also wrap `content` in `<think>`; but if it somehow does, the structured field still wins and `<think>` stripping is skipped to avoid double-capture).
  - `reasoning_content` absent/empty → fall back to the current `<think>` extraction (byte-for-byte current behaviour).
- Whitespace-only `reasoning_content` is treated as absent (`.filter(|s| !s.trim().is_empty())`), matching how `extract_think_block` rejects empty think bodies.
- The empty-`content` + non-empty-`reasoning_content` + `finish_reason: "length"` case (the GLM budget-exhaustion shape) must yield: `content` empty, `reasoning = Some(trace)`, `stop_reason = MaxTokens`. No new stop-reason logic — the existing `Some("length") => MaxTokens` mapping (`openai.rs:750`) already covers it; just confirm reasoning is attached.

### R3 — `reasoning()` accessor on `LlmResponse` (AC2)

In `crates/mika-common/src/llm/types.rs`, add a thin accessor so AC2's "via a method" is satisfied without callers reaching into the field:

```rust
/// Extended thinking / reasoning text, if the provider surfaced any.
pub fn reasoning(&self) -> Option<&str> {
    self.reasoning.as_deref()
}
```

Keep the `pub reasoning` field (existing callers at `agent_loop/mod.rs:854` and the OpenAI/Anthropic constructors read/write it directly). The accessor is additive surface, not a migration — do **not** churn existing field access sites.

### R4 — Persistence is inherited, not rebuilt (AC3)

No code change beyond R1+R2. AC3 is satisfied structurally: once `from_openai_response` populates `resp.reasoning`, the existing `agent_loop/mod.rs:854-859` path truncates and writes it to `llm_calls.reasoning` for every OpenAI-compatible provider. The DoD requires a **test** proving the field is populated end-to-end (unit-level: `from_openai_response` → `LlmResponse.reasoning`), not a live DB write. Document in the PR that the storage plumbing predates this ticket (schema v31, #653).

### R5 — Optional body-logging is already covered — note, don't build (ticket item 3, "Optional")

The dev-mode response-body log at `openai.rs:236-238` logs `body = ?resp` (the full `OpenAiResponse` `Debug`) under `MIKA_LOG_LLM_BODIES` / `mika::llm_debug`. Once `reasoning_content` is a field on the deserialized struct, it appears in that Debug output automatically. No dedicated log line is needed. State this in the PR; do not add a bespoke reasoning log statement (avoids duplicate/for-free work). If the reviewer wants an explicit `reasoning_len` INFO field on `llm_call completed` (`openai.rs:291`), that is a trivial add — flag as optional, not required.

## Verification contract

- `cargo test -p mika-common openai` — all existing `openai.rs` unit tests green (no regression to request serialization, cache parsing, `<think>` extraction, XML tool-call extraction, stop-reason mapping).
- **New unit tests in `openai.rs` `mod tests`:**
  1. `test_from_openai_response_parses_reasoning_content` — response with non-empty `reasoning_content` and non-empty `content` → `llm.reasoning == Some(trace)`, `llm.text() == content`.
  2. `test_from_openai_response_reasoning_content_empty_content_length_capped` — the GLM shape: `content: ""`, `reasoning_content: "<4507-char trace>"`, `finish_reason: "length"` → `llm.text().is_empty()`, `llm.reasoning == Some(trace)`, `llm.stop_reason == MaxTokens`, `usage.output_tokens > 0`.
  3. `test_from_openai_response_reasoning_content_precedence_over_think` — response with **both** a `reasoning_content` field and a `<think>` block in content → structured field wins, `<think>` not double-captured.
  4. `test_from_openai_response_reasoning_content_absent_falls_back_to_think` — no `reasoning_content`, `<think>` present in content → existing behaviour byte-for-byte (guards the fallback).
  5. `test_from_openai_response_reasoning_content_whitespace_treated_absent` — `reasoning_content: "   "` → treated as `None`, `<think>` fallback (if any) applies.
  6. `test_openai_request_never_serializes_reasoning_content` — build an `OpenAiRequest`, serialize, assert the JSON never contains `"reasoning_content"` (wire-compat guard for the shared-struct approach).
  7. `test_llm_response_reasoning_accessor` — `LlmResponse.reasoning()` returns the same `Option<&str>` as the field.
- **End-to-end deserialization test** (mirrors `test_from_openai_response_cache_details_json_deserialization`, `openai.rs:1591`): a raw Z.AI-shaped JSON payload string (empty `content`, populated `reasoning_content`, `completion_tokens_details.reasoning_tokens`, `finish_reason: "length"`) → `serde_json::from_str::<OpenAiResponse>` → `from_openai_response` → assert `reasoning` populated and `stop_reason == MaxTokens`. This proves the serde field name matches the real wire format.
- `cargo build && cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --check` clean.
- **AC4 verify-only:** `cargo test -p mika-agent calibration::failure` green (existing `test_classify_reasoning_budget_exhausted`, `test_reasoning_budget_requires_both_signals`) — proves the calibration signal AC4 references is intact and untouched.

## Definition of Done

- [ ] `OpenAiMessage` (or a response-only message struct) deserializes `reasoning_content: Option<String>` with `#[serde(default)]` and never serializes it into a request (R1).
- [ ] `from_openai_response` routes non-empty `reasoning_content` into `LlmResponse.reasoning` with the documented precedence over `<think>` extraction; `<think>` fallback preserved byte-for-byte (R2).
- [ ] The GLM budget-exhaustion shape (empty content + reasoning + `length`) yields `reasoning = Some`, empty text, `stop_reason = MaxTokens` (R2).
- [ ] `LlmResponse::reasoning()` accessor added; existing field-access callsites unchanged (R3).
- [ ] All seven new unit tests + the end-to-end JSON deserialization test pass; full existing `openai.rs` suite green (no regression to request serialization, cache, `<think>`, XML tool-calls) (Verification contract).
- [ ] `llm_calls.reasoning` population verified inherited (no new storage code); PR body notes the pre-existing schema-v31 pipeline (R4).
- [ ] Optional body-logging coverage noted in PR, not rebuilt (R5).
- [ ] AC4 confirmed already-shipped via mika#1665 and untouched; existing calibration failure tests green (PR body cites them).
- [ ] `cargo build`, `cargo clippy -D warnings`, `cargo fmt --check` all clean.
- [ ] PR body records the R1 struct-shape decision (shared field + skip-serializing vs. response-only struct) and the AC-by-AC done/remaining split above.

## Acceptance criteria

Transcribed verbatim from mika#1664:

- **AC1** — `OpenAiResponse` struct in openai.rs parses `message.reasoning_content` as `Option<String>`.
- **AC2** — `LlmResponse` exposes the reasoning via a method (`reasoning()` or via `thinking()`).
- **AC3** — `llm_calls.reasoning` column is populated when reasoning_content is present.
- **AC4** — Calibration harness's classify_failure code treats "empty content with non-zero reasoning_tokens" as a distinct failure class (`ReasoningBudgetExhausted`) instead of generic EmptyResponse.

> **Status note (grounded):** AC4 is **already satisfied** by prior work (mika#1665) — `FailureClass::ReasoningBudgetExhausted` + the extended `classify_failure` signature + `empty_response_result` helper are present with tests. This PR treats AC4 as verify-only (no code change) and closes it by citing the existing coverage. AC1 is the only unimplemented criterion; AC2 (field present, add accessor) and AC3 (storage path present, add test) are satisfied by minimal additive work riding on AC1.

## Out of scope

- Suppressing reasoning mode at request time (a `thinking: false` flag on `LlmRequest` to disable reasoning on supporting providers) — separate enhancement.
- The calibration scenario's `max_tokens=1000` cap — separate ticket (mika#1665 lineage).
- Extending `reasoning_content` parsing to the native Ollama provider (`llm/ollama.rs`) — the ticket scopes the OpenAI-compatible adapter only. Ollama's `<think>` handling is unaffected.
- Any change to AC4's calibration code (already shipped; touching it risks regressing #1665).
- Dashboard/UI surfacing of the reasoning column — persistence only, per ticket.

## Risks & mitigations

- **R1 shared-struct wire regression:** adding a field to the request-and-response `OpenAiMessage` could leak `reasoning_content` into outbound requests. *Mitigation:* `skip_serializing_if = "Option::is_none"` + the explicit `test_openai_request_never_serializes_reasoning_content` guard; response-only-struct fallback documented if the guard is judged insufficient.
- **Precedence ambiguity vs. `<think>`:** a provider emitting both channels could double-capture reasoning. *Mitigation:* structured field wins and short-circuits `<think>` stripping; test #3 pins the behaviour.
- **Serde field-name drift:** if Z.AI's real key differs from `reasoning_content`, the field silently stays `None`. *Mitigation:* the end-to-end raw-JSON deserialization test uses the exact wire shape from the ticket's captured evidence, catching a name mismatch at test time.
