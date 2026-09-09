---
title: "fix(llm): classify a whitespace-only response body as retryable Transport (#1781)"
type: fix
status: active
date: 2026-09-09
origin: senara-solutions/mika#1781
---

# fix(llm): classify a whitespace-only response body as retryable Transport (#1781)

## Overview

A response body made **entirely of whitespace** — no JSON character at all — currently reaches
`serde_json::from_str` and comes back as `LlmError::ParseError`, which `is_retryable()` rejects.
Functionally that body is an empty response, so it is transient and belongs on the retry path,
exactly where #2016 put body-read failures.

This is the **residual** of a ticket that carried something much bigger. #2015 made the failure
diagnosable and #2016 routed the dominant class (mid-stream read failures) to `Transport`. The
measurement below shows those two closed the throughput blocker; what is left is one narrow
misclassification, observed once.

## Problem frame

`crates/mika-common/src/llm/openai.rs:277` reads the body as text (that is #2015's change), then
line 299 deserializes it:

```rust
let body = match response.text().await { /* Err => LlmError::Transport (#2016) */ };

let resp: OpenAiResponse = serde_json::from_str(&body).map_err(|e| {
    // ... warn! with serde error, body_len, capped excerpt ...
    LlmError::ParseError(/* ... */)
})?;
```

`LlmError::is_retryable()` (`crates/mika-common/src/llm/error.rs:32-38`) returns `true` for
`Transport` and for `HttpError { retryable: true }`, and `false` for every `ParseError`. That
`false` is right for genuinely malformed JSON — retrying a schema mismatch just burns the deadline.
It is wrong for a body that contains no JSON to be malformed.

### The one observation, fully characterised

Captured by #2015's diagnostics, provider `openrouter`, 2026-08-28T20:53:23Z:

```
serde:        EOF while parsing a value at line 241 column 0
body_len:     1320
body_excerpt: 1320 bytes of newlines and spaces only — zero JSON characters
```

The body **arrived whole** — no truncation. 240 blank lines. Not a cut stream, not an HTML error
page, not a changed schema: the shape of a keepalive or an upstream timeout where a proxy pads the
connection while a backend expires.

### What the measurement says today (2026-09-09)

From `/var/log/mika/server.log`. Raw counts are halved before 2026-09-06 — every line was written
twice until mika#2195/PR#2210 fixed it.

| Signal | Result |
|--------|--------|
| `WARN resume_agent run failed` carrying a parse error | last occurrence **2026-08-28**; zero in the 11 days since |
| `ERROR agent loop failed` carrying a parse error | last occurrence **2026-08-27** |
| #2016 transport branch (`body read failed mid-stream`) | **~25–56/day**, still firing, fully absorbed by retry — no cycle killed since 2026-08-29 |
| #2015 serde branch (`body did not parse`) | **1 occurrence, ever** — the one above |

So the p1 throughput blocker is closed. **n=1 in 12 days** for what remains.

The reason to fix it anyway is the invariant, not the volume: *a cycle cannot be counted as
successful when its output is empty or unreadable* (Prime bearing, RT#009). An empty response must
be retried or surface loudly — never kill the turn quietly. Five lines of classification buy that
guarantee permanently; leaving it means the next proxy-padding episode silently costs a pilot cycle
again.

## Change

One guard in `send_once`, between the text read and the deserialization, plus the pure predicate it
delegates to so the decision is unit-testable without an HTTP mock.

**1. A pure classifier next to `send_once`:**

```rust
/// A body carrying no JSON at all is functionally an empty response, hence
/// transient — not a parse failure (mika#1781).
///
/// Returns `Some(Transport)` for a body that is empty or whitespace-only, so it
/// routes through the fast-retry path (#1744). Returns `None` for anything that
/// should go on to serde, including genuinely malformed JSON, which keeps
/// `ParseError` and its diagnostics from #2015.
fn whitespace_only_body_error(body: &str) -> Option<LlmError> {
    body.trim().is_empty().then(|| {
        LlmError::Transport(format!(
            "empty (whitespace-only) response body, {} bytes",
            body.len()
        ))
    })
}
```

**2. The call site in `send_once`, immediately after the body is read (line ~296):**

```rust
if let Some(err) = whitespace_only_body_error(&body) {
    warn!(
        target: "mika::llm",
        provider = %self.provider_kind,
        body_len = body.len(),
        "LLM response body was whitespace-only (retryable transport)"
    );
    return Err(err);
}
```

The `warn!` stays at the call site because it needs `self.provider_kind`; the predicate stays pure
so the classification can be tested directly.

**3. Tests** in `crates/mika-common/src/llm/openai.rs`'s existing `mod tests`, and one in
`error.rs` if the retryability assertion reads better there.

### Scope discipline

The fix **classifies**; it does not change retry policy, its bounds, or its backoff — #1744 owns
that path. Success path untouched. Every other error variant untouched. A malformed-but-non-blank
body keeps `ParseError` and the full serde diagnostic from #2015.

### Why a pure predicate rather than an end-to-end test

`mika-common` has no HTTP mock in its dev-dependencies (`tokio`, `tempfile`, `serial_test`), and
`send_once` is private and takes an `OpenAiRequest`. Adding `wiremock` — available at the workspace
level, already a dev-dep of `mika-agent` and `mika-gateway` — would let a test drive the whole
function, but the transport layer is not what changes here. The decision under test is "blank body
→ Transport, malformed body → ParseError", and a pure predicate tests exactly that with no new
dependency. The consequence, stated plainly: **AC2's `warn!` is verified by review, not by the test
suite** — the tests cover the classification and its retryability, not the log line.

## Acceptance criteria

- **AC1** — In `send_once` (`crates/mika-common/src/llm/openai.rs`), after the body is read as text
  and **before** `serde_json::from_str`, a body whose `trim()` is empty returns
  `LlmError::Transport` naming the byte length, and never `ParseError`.
- **AC2** — That case emits a dedicated `warn!` on target `mika::llm` carrying `provider` and
  `body_len`, distinct from #2015's `"LLM response body did not parse"` — the two classes stay
  separable in the logs.
- **AC3** — A genuinely malformed (non-blank) JSON body keeps `LlmError::ParseError` with its full
  serde diagnostic — line, column, `body_len`, capped excerpt. No regression on #2015.
- **AC4** — Unit tests: a body of 1320 newline/space characters (the exact shape observed on
  2026-08-28) yields `Transport` with `is_retryable() == true`; a `{bad` body yields `ParseError`
  with `is_retryable() == false`. Both assert the classification *and* the retryability, since the
  retryability is the whole point.
- **AC5** — No change to the success path, to any other error variant, or to retry policy itself.
  `cargo clippy --all-targets -- -D warnings` and `cargo test -p mika-common` pass.

## Fire-Disposition

Both deliverables that can fire are named here with what happens when they do (mika#1574).

- **AC4 — unit tests (CI).** Fire on: the diff / CI. Disposition: **blocking CI gate**. The
  whitespace test must be verified **red before** the guard lands — on today's code a 1320-byte
  blank body reaches `serde_json::from_str` and yields `ParseError`, so the assertion
  `is_retryable() == true` fails. Green only after the guard. No auto-remediation.
  There is **no pre-existing-violation backlog to dispose of**: the classifier is new code on a new
  path, not a sweep over existing call sites, so the "land disabled then clean up" option has an
  empty set to work on and is not needed.
- **AC2 — the `warn!` as a runtime probe (operator).** Fire on: `mika::llm` in
  `/var/log/mika/server.log` after deploy. Disposition: **halt-and-surface, not auto-retry-forever**.
  A `"LLM response body was whitespace-only"` line is expected and benign on its own — it means the
  class was caught and routed. What is *not* benign is that same `task_id` appearing afterwards in a
  `WARN resume_agent run failed`: that would mean the retry path did not absorb it, and the fix
  routed the error without changing the outcome. In that case stop and re-open, do not re-deploy.
  Stated honestly: with a base rate of n=1 in 12 days, silence on this probe is **not** evidence the
  fix works. The red-before-green unit test is the evidence; this probe only confirms the routing if
  and when the class recurs.

## Out of scope

- Repairing malformed JSON in any form (schema-gated repair, third-party "suture" libraries). A
  non-blank invalid body stays an error.
- Splitting `ParseError` into per-stage variants (`ResponseEnvelopeParse` / `ToolArgumentsParse`),
  as the external comment of 2026-08-27 proposed. Post-#2015 measurement shows the envelope-serde
  branch fired **once** and the tool-arguments stage **never**. Refactoring the error type for n=1
  is not carried by the evidence; if the serde branch takes on volume in a non-blank shape, that is
  a separate ticket with its own measurement.
- Changing retry bounds, backoff, or deadline arithmetic (#1744).
- Persisting a typed failed cycle and surfacing it loudly on exhausted retries — point 3 of the
  external comment of 2026-08-27. It is a real gap and a defensible one to close, but it sits in the
  task engine's cycle-completion contract, not at the LLM response boundary, and it is not what the
  measurement here points at. It belongs in a ticket of its own against `mika_agent::task_engine`.

- Capturing raw bodies as fixtures. #2015 settled that posture: capped excerpt, on failure only.

## Verification

1. `cargo test -p mika-common llm::` — new tests pass, existing ones unchanged.
2. `cargo clippy --all-targets -- -D warnings`.
3. Read the diff against AC3: the serde `map_err` block from #2015 is untouched.
4. Post-merge, the log check that would show the fix working:
   `grep 'whitespace-only' /var/log/mika/server.log` — expected to stay empty for long stretches
   (n=1 in 12 days), and when it does fire, the same `task_id` must **not** appear in a subsequent
   `resume_agent run failed`. Note honestly: with a base rate this low, absence of the failure is
   not by itself evidence the fix works. The unit test is the evidence; the log check is the
   confirmation that the class was routed, if and when it recurs.

## Files touched

- `crates/mika-common/src/llm/openai.rs` — the guard, the predicate, the tests.
- `crates/mika-common/src/llm/error.rs` — no change expected; listed because AC4's retryability
  assertion may read better beside the existing `all_transport_errors_are_retryable` test.
