---
issue: 1660
type: fix
date: 2026-06-30
---

# Plan — fix(llm): hardcoded 120s HTTP timeout in openai.rs blocks long-context synthesis (mika#1660)

## Problem

`crates/mika-common/src/llm/openai.rs:160` and `crates/mika-common/src/llm/ollama.rs:219` hardcode `Duration::from_secs(120)` as the `reqwest` HTTP-client timeout. The timeout covers the full request lifecycle (including streamed generation), so any synthesis whose wall-clock exceeds 120s aborts with a transport-layer timeout.

Hard evidence (2026-06-29, glm-5.2 native + via OpenRouter):
- 7 of 8 measured skill-review runs completed in 42–68s (all 1.6–12.4 KB input/output).
- The `self-dev` skill (54 KB source, ~27 KB required adapted output) timed out at 139s (native) and 120s (OpenRouter).
- The cliff sits between 12 KB and 27 KB of output; long-context synthesis cannot fit in 120s for any provider tested.

This blocks skill-review on `self-dev` for any glm-5.2 variant — and `self-dev` is mika-dev's brain. Any future high-context skill or long-tail model swap hits the same wall.

## Architectural lineage

- mika#1657 — Z.AI native provider; the implementation that surfaced this constraint at the faster-than-OpenRouter wall-clock.
- mika#1633 — glm-5.2 swap; introduced the long-tail synthesis behavior that exceeds 120s for ≥20 KB outputs.

## Fix shape

Single env-var override at provider construction. The constant becomes a default; `MIKA_LLM_HTTP_TIMEOUT_SECS` overrides it globally per process. Both providers read the same env var so they stay in lockstep. Per-skill timeouts (issue body §Solution shape Option 2) are out of scope — file a follow-up if the env-var ceiling needs refinement.

The retry deadline math (`TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` at `mod.rs:24+26`, used at `openai.rs:268,316` and `ollama.rs:525,572`) is **separate from** the HTTP timeout. `TYPICAL_CALL_DURATION_SECS = 90` is the budget estimate for "is there time to retry?" — it does not need to scale with the HTTP timeout. Operators raising the timeout to 600s know they're trading retry-budget headroom for absolute deadline; the retry math gracefully degrades (fewer retries fit in the budget) but doesn't break. Out of scope: changing retry math, unless review surfaces a hidden coupling.

## Implementation outline

0. **Pre-commit survey (architect F-Q3):** grep `crates/mika-common/src/llm/` for an existing `LlmConfig` or `Config` struct. If found, implement the helper as `LlmConfig::http_timeout()` and route call sites through that struct. If none exists, `mod.rs` is the correct home (per Step 1 below). Decision made at implementation time; either branch satisfies AC2.

1. **New helper in `mika-common/src/llm/mod.rs`** (or on existing `LlmConfig` per Step 0): `pub fn http_timeout_secs() -> u64` — reads `MIKA_LLM_HTTP_TIMEOUT_SECS`, defaults to 120 when unset/empty, panics with a clear message on values `< 10` or unparseable. Panic at provider construction (cold-path startup) is the fail-fast pattern; silent fallback to 120s would mask config errors until the next long-context timeout.
2. **`openai.rs:160`** — replace `Duration::from_secs(120)` with `Duration::from_secs(http_timeout_secs())`.
3. **`ollama.rs:219`** — same swap.
4. **Unit tests in `mod.rs`** (or wherever the helper lives):
   - default case (env unset → 120)
   - valid override (env="600" → 600)
   - too-small rejection (env="5" → panic / Err with message)
   - unparseable rejection (env="abc" → panic / Err with message)
5. **No regression of default behavior:** existing call sites consume the helper; default-of-120 covered by AC4.

The helper's exact return shape (panic vs Result) is a small judgment call deferrable to the implementer — both satisfy AC1's "rejected with a clear error" — but **panic at construction** is the pattern of least surprise here (provider construction is non-recoverable startup work; misconfigured env var should crash loud, not silently fall back to default).

## Acceptance criteria

- **AC1** — `MIKA_LLM_HTTP_TIMEOUT_SECS` env var read at provider construction; defaults to 120 when unset; values `< 10` rejected with a clear error (panic at construction, message naming the env var and the offending value).
- **AC2** — Same env var flows through both `openai.rs` and `ollama.rs` provider constructors via a single shared helper in `llm/mod.rs`.
- **AC3** — Smoke test: `MIKA_LLM_HTTP_TIMEOUT_SECS=600 mika ask --agent mika-dev "use review_skill to adapt self-dev"` completes (no transport-layer timeout) when the underlying model can finish in <600s. Documented in PR body with `trace_id: <uuid>, wall_clock: <Ns>, output_tokens: ~<N>k, status: success under 600s ceiling` so AC3 is independently verifiable post-merge without database lookup (architect F-Q6).
- **AC4** — No regression on existing 120s default behavior. Unit tests cover default-unset and override paths; existing integration tests (`cargo test -p mika-common`) stay green.

## Out of scope

- Per-skill `http_timeout_secs` declaration in `SkillManifest` (issue body §Solution shape Option 2) — separate follow-up if the env-var path needs refinement.
- `TYPICAL_CALL_DURATION_SECS` / retry deadline math reshuffling — see §Fix shape rationale.
- Skill-review variant-shrinking rule — different lever, leave it alone.
- Any change to `ollama.rs` 's local-model deployment story.

## Files involved

- `crates/mika-common/src/llm/mod.rs` — new `http_timeout_secs()` helper + unit tests
- `crates/mika-common/src/llm/openai.rs:160` — swap hardcoded `120` for helper call
- `crates/mika-common/src/llm/ollama.rs:219` — swap hardcoded `120` for helper call

## Verification

- `cargo test -p mika-common` — unit tests for helper, including default + override + too-small + unparseable cases.
- `cargo build --release` — full build clean.
- Smoke (AC3): with `MIKA_LLM_HTTP_TIMEOUT_SECS=600` set on mika-dev, run skill-review on `self-dev` (the 54 KB founding case). Expect completion under 600s with no transport timeout. Capture trace_id in PR body.
- Default-path regression: without `MIKA_LLM_HTTP_TIMEOUT_SECS` set, run a baseline `mika ask --agent mika-dev "status"` and confirm behavior identical to pre-fix main (120s default).
- Negative test (manual or scripted): `MIKA_LLM_HTTP_TIMEOUT_SECS=5 mika ask ...` — expect provider construction panic with the env-var name and value in the error message.

## References

- mika#1657 — Z.AI native provider (surfaced constraint at faster wall-clock)
- mika#1633 — glm-5.2 swap (introduced long-tail synthesis behavior)
- Body bytes: `~/.mika/agents/mika-dev/logs/mika.log.2026-06-29`, trace_id `99cee3e0bf4f4be28aaa8cfceed83f49` — first observed timeout (skill-review on self-dev, OpenRouter routing)
- `crates/mika-common/src/llm/mod.rs:24,26` — `TYPICAL_CALL_DURATION_SECS` + `RETRY_BUFFER_SECS` constants (the retry-deadline math; intentionally not touched by this fix)
- `crates/mika-common/src/llm/openai.rs:160`, `crates/mika-common/src/llm/ollama.rs:219` — the hardcoded constants this fix replaces
