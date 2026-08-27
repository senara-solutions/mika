---
title: "fix: make the LLM response parse failure diagnosable (#1781)"
type: fix
status: active
date: 2026-08-27
origin: senara-solutions/mika#1781
---

# fix: make the LLM response parse failure diagnosable (#1781)

## Overview

`LLM response parse error: failed to parse response: error decoding response body` has fired **7138 times since 2026-04-06** and continues at 12–42 occurrences per hour. Each occurrence kills a pilot cycle: the agent loop fails after grooming, so branches accumulate plan markdown and no code. It is the dominant throughput blocker on the dev loop today.

The ticket sat in p2 for four months labelled "graceful fallback correct, upstream flake". This plan does not fix the flake. **It fixes the reason nobody could tell whether it was a flake** — the error, as logged, contains nothing to diagnose.

## Problem frame

`crates/mika-common/src/llm/openai.rs:230` deserializes the response with:

```rust
let resp: OpenAiResponse = response
    .json()
    .await
    .map_err(|e| LlmError::ParseError(format!("failed to parse response: {e}")))?;
```

`response.json()` **consumes** the body to deserialize it. When deserialization fails, reqwest surfaces the opaque string `error decoding response body` and the bytes are gone. Nothing downstream can distinguish between:

- a truncated stream (connection dropped mid-body),
- an HTML error page from a proxy or gateway,
- a JSON error object the provider returned with HTTP 200,
- a schema the provider changed under us (a renamed or newly-nullable field),
- a body that is valid but exceeds what `OpenAiResponse` models.

Those five causes have five different fixes, ranging from "retry" to "add a field". Four months of logs cannot tell them apart, and no amount of further log-counting will: the information was never captured.

The success path already logs its body under `mika::llm_debug`, but that target is dev-gated (`MIKA_LOG_LLM_BODIES`) and off in production, where the failures actually happen.

## Change

Read the body as text first, then deserialize from the string. On failure, log the serde error (which names line, column and offending field — unlike the reqwest error it replaces), the body length, and a capped excerpt.

The error returned to the caller also carries the length and a short prefix, so the failure is legible in an aggregated log line without needing the full record.

### Scope discipline

**This is observability only. No behaviour changes.** Same success path, same `LlmError::ParseError` variant, same retry semantics, same call sites. Once a week of logs identifies which of the five causes is dominant, the actual fix becomes a separate ticket with evidence behind it rather than a guess.

### On logging response bodies

The excerpt is capped at 400 characters and emitted **only on parse failure**, at `warn!` on the `mika::llm` target.

For an OpenAI-compatible response the head of the body is metadata — `id`, `object`, `created`, `model` — and completion text appears later, past the cap in any realistic response. When the body is an error page or an upstream JSON error, the head is precisely what needs reading. The residual risk is a malformed body whose first 400 characters happen to contain user content; that is judged acceptable against four months of undiagnosable failures, and the cap keeps the exposure bounded. If the risk is judged unacceptable in review, the alternative is logging `body_len` and the serde error alone — that still separates "truncated" from "schema mismatch", which is most of the value.

## Acceptance criteria

- **AC1** — `send_once` reads the response body via `response.text()` and deserializes with `serde_json::from_str`, so the body survives a parse failure.
- **AC2** — On parse failure, a `warn!` on target `mika::llm` records: `provider`, the serde error, `body_len`, and a `body_excerpt` capped at 400 characters.
- **AC3** — The returned `LlmError::ParseError` message includes the body length and a prefix of at most 120 characters, so a single aggregated log line is enough to classify the failure.
- **AC4** — No behaviour change on the success path: same deserialized `OpenAiResponse`, same error variant on failure, no change to retry or fallback logic. `cargo check`, `cargo clippy` and `cargo fmt --check` clean on `mika-common`.
- **AC5** — Post-deploy verification: within 24 hours of deploy, `grep "LLM response body did not parse" /var/log/mika/server.log` yields records that name the failing field or show the body's shape, and #1781 can be re-triaged against evidence instead of a guess.

## Out of scope

- Fixing the underlying parse failure. That needs the evidence this change produces.
- The same pattern in `github_graphql.rs:198` and `tools/check_task.rs:115` — same shape, but neither is on the pilot's hot path, and mixing them in dilutes the AC5 signal.
- Streaming responses, which take a different path.
