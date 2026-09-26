---
title: "fix: classify LLM body-read failures as retryable transport errors (#1781)"
type: fix
status: active
date: 2026-08-27
origin: senara-solutions/mika#1781
---

# fix: classify LLM body-read failures as retryable transport errors (#1781)

## Overview

This is the evidence-based fix that #2015 (observability) was built to enable — landing the same day because the evidence arrived within the first hour.

**Measured after deploying #2015's diagnostics (2026-08-27 09:52Z): 48 of 48 parse-error occurrences were body-read failures.** Every error message carries the new `failed to read response body` marker from the `response.text()` branch; the serde branch — which would indicate an HTML error page, an upstream JSON error on 200, or a provider schema change — **never fired once**. The five-way ambiguity documented in #2015's plan is resolved: the dominant (so far: only) cause is the connection failing mid-body. The bytes never arrive.

## Problem frame

A mid-body read failure is a **network failure**, but `send_once` maps it to `LlmError::ParseError` — and `LlmError::is_retryable()` returns `false` for `ParseError`. So the retry loop at `openai.rs:353` (`attempt < MAX_RETRIES && e.is_retryable()`) gives up immediately, the agent loop fails, and the pilot cycle dies.

The machinery to handle this correctly already exists and is proven: `LlmError::Transport` is retryable, with a dedicated fast-retry deadline carve-out (#1744) built precisely for transport failures that resolve in seconds. The z.ai transport wedge that killed mika-qa's 2026-07-07 turn was the same class. This failure mode was simply never routed there — before #2015, it was indistinguishable from a real parse failure.

Consequence over four months: 7138 killed cycles, most or all of which a single retry would likely have absorbed. This is the mechanism behind "the loop grooms but never delivers" (#1901's symptom).

## Change

In `send_once` (`crates/mika-common/src/llm/openai.rs`), the `response.text()` error branch:

1. maps to `LlmError::Transport` instead of `LlmError::ParseError` — making it retryable via the existing loop and eligible for the #1744 fast-retry threshold;
2. walks the reqwest error's `source()` chain into the message, because the top-level Display is the opaque `error decoding response body` while the actual cause (unexpected EOF, decompression failure, connection reset) lives one or two levels down;
3. keeps a `warn!` on target `mika::llm` so the frequency of transport failures stays observable after they stop killing cycles.

The serde branch from #2015 is untouched: a body that **arrives** but does not parse is still a genuine non-retryable parse failure with full diagnostics.

## Acceptance criteria

- **AC1** — `response.text()` failure returns `LlmError::Transport`, and `is_retryable()` / `is_transport()` both return `true` for it (existing impl, no change needed there).
- **AC2** — The error message includes the full reqwest source chain, not just the top-level Display.
- **AC3** — A `warn!` on target `mika::llm` records provider and the chained error on every body-read failure.
- **AC4** — The serde-failure branch (#2015) is behaviourally unchanged: still `ParseError`, still non-retryable, still logs `body_len` and excerpt.
- **AC5** — `cargo check`, `cargo clippy`, `cargo fmt --check` clean on `mika-common`.
- **AC6** — Post-deploy: `agent loop failed` occurrences with `LLM response parse error` drop to near zero (from 12–42/hour), while `LLM response body read failed mid-stream` warns continue to appear and are absorbed by retries. If parse errors persist at rate after this lands, the serde branch's diagnostics identify the residual cause.

## Out of scope

- Root-causing WHY the upstream connection drops mid-body (openrouter/GLM side, or local egress proxy). The warn frequency after this fix is the input to that investigation.
- Retry-budget tuning; the existing MAX_RETRIES and #1744 thresholds apply as-is.
- The same mapping in streaming paths, `github_graphql.rs`, `check_task.rs` — different call sites, separate evidence needed.
