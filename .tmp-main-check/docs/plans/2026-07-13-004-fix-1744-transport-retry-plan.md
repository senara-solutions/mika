---
type: fix
issue: 1744
title: Transport-class retry uses smaller deadline threshold (mika#1744 AC4-primary)
status: draft
---

# Plan — mika#1744 transport-class retry threshold

## Ticket

mika#1744 — mika-qa's 2026-07-07 turn was killed when a z.ai transport error
struck ~2min *after* the 5-min deadline had already expired. The retry loop
bailed because remaining budget was less than
`TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` (120s), even though a
transport-class retry resolves in seconds — not the full per-request timeout
that governs HTTP-status errors. AC4 (transport resilience) is the primary,
highest-leverage fix; the empirical AC1 measurement (max 45.4% context usage,
≥55% headroom) demoted AC2 to opportunistic hygiene.

## Problem

Both the Claude client (`crates/mika-common/src/claude.rs`) and the
OpenAI-compatible provider (`crates/mika-common/src/llm/openai.rs`) share a
symmetric deadline-aware retry-abort branch. Before this fix, the abort
threshold was a single constant (`TYPICAL_CALL_DURATION_SECS +
RETRY_BUFFER_SECS = 120s`) regardless of the last error's class. A
transient DNS / TLS / socket-reset failure resolves in seconds; abandoning
the retry chain because "another 120s worth of budget isn't available" is
the wrong shape for that error class and directly caused the 2026-07-07
kill.

## Scope

**In scope (v1 ships):**

1. New `TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` constant in
   `crates/mika-common/src/llm/mod.rs`. Doc-comment names mika#1744 and
   explains the 60s = typical-call + 30s-slack rationale.
2. New `LlmError::is_transport()` classifier in
   `crates/mika-common/src/llm/error.rs`. Returns true only for the
   `Transport(_)` variant. Unit tests assert (a) only `Transport` returns
   true, (b) all transport errors are retryable, (c) HTTP 500 is retryable
   but not transport (independence guardrail).
3. Retry-loop threshold selection in both providers. When the last error
   was transport-class, use `TRANSPORT_RETRY_MIN_REMAINING_SECS`; otherwise
   preserve the existing 120s threshold. Applied symmetrically to Claude
   (`ClaudeApiError::Transport`) and OpenAI-compatible
   (`LlmError::is_transport`) paths.
4. Diagnostic-classification symmetry. The post-loop `deadline_aborted`
   computation uses the same transport-aware threshold so the
   deadline-abort surface classifies transport-late failures correctly
   instead of misreporting them as "max retries exceeded".
5. Structured logging on abort. `warn!` gains `threshold_secs` and
   `last_was_transport` fields so post-hoc forensics can distinguish
   which threshold fired without re-reading the code.

**Out of scope (tracked separately in ticket body):**

- AC3 — extending `AGENT_TOTAL_TIMEOUT_SECS` from 300s. Separate axis;
  ships behind its own PR after empirical justification.
- AC2 — system_prompt compression. Downgraded per AC1 measurement.
- AC5 — end-to-end verification re-firing samidarko-claude's original
  direct-ask. Requires deploy + live-observation, tracked as a follow-up.
- Fallback provider routing on transport failure. Larger design; not part
  of this fix's mechanism.

## Committed positions

1. **60s is the right threshold.** Typical call (90s) minus buffer (30s)
   was the prior computation for the full-timeout class; for transport
   the class-appropriate figure is one typical-call duration with matching
   slack — the retry only needs to complete once, and transport failures
   resolve fast.
2. **Both providers must move in lockstep.** The Claude and OpenAI-compat
   retry loops are structurally symmetric; splitting the discipline would
   let one provider silently kill turns that the other saves. Both paths
   ship in the same PR, both consume the same constant.
3. **`is_transport()` lives on `LlmError`, not per-provider.** The
   OpenAI-compatible path uses the trait method; the Claude path pattern-
   matches on `ClaudeApiError::Transport(_)` because it has its own error
   type. Symmetric behaviour, provider-shape-appropriate mechanics.
4. **Classifier independence is load-bearing.** `is_transport()` and
   `is_retryable()` must be independent — all transport errors are
   retryable but HTTP 429/500 are retryable and NOT transport. Explicit
   test coverage prevents a future refactor from conflating them.

## Acceptance criteria

- [ ] **AC1** — `TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` added to
  `crates/mika-common/src/llm/mod.rs` with a doc-comment naming mika#1744
  AC4-primary and the 60s rationale.
- [ ] **AC2** — `LlmError::is_transport()` added to
  `crates/mika-common/src/llm/error.rs`. Returns true only for
  `LlmError::Transport(_)`.
- [ ] **AC3** — Unit tests in `error.rs` cover: (a) only `Transport`
  returns true; (b) transport errors are also retryable; (c) HTTP 500 is
  retryable but not transport (independence guardrail).
- [ ] **AC4** — `crates/mika-common/src/llm/openai.rs` retry loop selects
  `TRANSPORT_RETRY_MIN_REMAINING_SECS` when the last error is
  transport-class, otherwise preserves the existing
  `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` threshold.
- [ ] **AC5** — `crates/mika-common/src/claude.rs` retry loop applies the
  same transport-aware threshold selection, pattern-matching on
  `ClaudeApiError::Transport(_)`.
- [ ] **AC6** — `deadline_aborted` post-loop diagnostic uses the same
  transport-aware threshold in both providers so the abort-classification
  surface matches whichever threshold actually fired.
- [ ] **AC7** — `warn!` on retry-abort in both providers includes
  `threshold_secs` and `last_was_transport` fields for forensics.
- [ ] **AC8** — `cargo test -p mika-common` clean; new `is_transport()`
  tests pass; no regression in existing retry / claude / openai tests.
- [ ] **AC9** — `cargo clippy` and `cargo fmt --check` clean.

## Definition of Done

- All acceptance criteria above are checked off.
- CI green on the PR: `cargo build`, `cargo test`, `cargo clippy`,
  `cargo fmt --check`, byte-slice lint, loop-select lint, docker-build,
  pr-body-validation, verify-bundled-skills, pipeline-artifacts.
- PR body references mika#1744 AC4-primary with an explicit note that
  AC3/AC5 remain open on separate tracks and AC2 is downgraded.
- No unrelated changes: this PR is scoped to the transport-threshold fix
  plus its tests; no deadline change (AC3) and no compression (AC2) leak
  in.

## References

- Ticket: mika#1744 — qa-review system_prompt at 91% context budget (AC4
  transport-resilience primary).
- Parent architecture: mika#1727 — daemon-agent architecture (this fix is
  an orthogonal substrate patch).
- Related timeout constant: `AGENT_TOTAL_TIMEOUT_SECS = 300` at
  `crates/mika-agent/src/planning/policy.rs:18` (AC3 axis, out of scope
  here).
- Prime persistence 2026-07-07 13:15 CEST:
  `enforcement-mechanism-must-obey-its-own-discipline` — the substrate
  that enforces review discipline must not itself violate that discipline
  by killing disciplined answers.
- MPC forensic 2026-07-07 11:08 CEST identifying the transport-late
  compounding as the actual failure mode.
