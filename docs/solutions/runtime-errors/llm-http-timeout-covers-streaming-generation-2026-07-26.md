---
module: mika-common
tags: [llm, reqwest, timeout, streaming, long-context, skill-review, glm-5.2]
problem_type: reliability
category: llm-providers
---

# reqwest HTTP timeout covers the whole generation, not just connect (mika#1660)

## Problem

`OpenAiCompatibleProvider` and `OllamaProvider` both hardcoded a 120s
`reqwest::ClientBuilder::timeout(...)`. `reqwest`'s request timeout bounds the
**entire request lifecycle** — connect, send, *and* the streamed generation
phase — not just connection setup. So any LLM call whose wall-clock exceeds
120s aborts with a transport-layer timeout, regardless of how healthy the
connection is.

This surfaced when glm-5.2 (mika#1633) started producing long-tail synthesis.
Empirically the cliff sat between 12 KB and 27 KB of output: skill-review on
the 54 KB `self-dev` skill (~27 KB required adapted variant) hit 139s on Z.AI
native and 120s via OpenRouter — both timed out. `self-dev` is mika-dev's
brain, so it could not be skill-reviewed for any glm-5.2 variant.

## Root cause

A single hardcoded `Duration::from_secs(120)` at each provider's client-build
site. Not configurable, and set at a value tuned for short calls. Faster
providers (native Z.AI vs OpenRouter relay) do not help enough — the bottleneck
migrated from "OpenRouter margin" to "HTTP timeout" once the model's output
grew.

## Fix

Made the timeout configurable via `MIKA_LLM_HTTP_TIMEOUT_SECS`, read through a
single shared helper so both providers stay in lockstep:

- `mika_common::llm::http_timeout_secs()` — reads the env var, defaults to 120
  when unset/empty, **panics** at provider construction on unparseable values
  or values `< 10`. Fail-fast is deliberate: provider construction is cold-path
  startup, and a silent fallback to 120 would mask the misconfiguration until
  the next long-context call timed out.
- The parse/validate logic lives in a pure `parse_http_timeout(Option<&str>)`
  so it is unit-testable without mutating the process-global env var (which
  would race parallel tests — edition 2024 also makes `set_var` `unsafe`).

## Gotchas / boundaries

- **Not the same as the retry-deadline math.** `TYPICAL_CALL_DURATION_SECS`
  (90) + `RETRY_BUFFER_SECS` (30) in `llm/mod.rs` govern the deadline-aware
  retry abort ("is there time to retry?"), *not* the HTTP timeout. They are
  intentionally decoupled: raising the HTTP timeout to 600s trades retry-budget
  headroom for absolute deadline, but the retry math degrades gracefully
  (fewer retries fit) rather than breaking. Do not synchronize them.
- **Anthropic is unaffected** — it uses its own native client
  (`anthropic.rs`), not the OpenAI-compatible/Ollama transports.
- **`models.rs` model-list fetch** has its own `FETCH_TIMEOUT_SECS` and is out
  of scope — that is a short metadata call, not a generation.

## Testable pattern worth reusing

For any env-var-driven config with validation, split the impure env read from
the pure parse/validate function. The public fn does
`parse(std::env::var(KEY).ok().as_deref())`; tests drive the pure fn with
`Option<&str>` inputs and `#[should_panic(expected = "KEY")]` for rejections —
no global-state mutation, no serial-test constraint.
