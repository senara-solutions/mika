---
module: mika-agent
tags: [timeout, observability, agent-loop]
problem_type: runtime-error
ticket: mika#848
date: 2026-04-27
---

# Agent total-timeout cancels in-flight LLM calls without diagnostic trail

## Symptom

Long mika sessions occasionally end with the canned message
`"I'm sorry, that took too long. Let me try a simpler approach next time."` —
and zero indication of what was happening at the moment the timeout fired.
The dashboard's `llm_calls` table shows N successful rows, then a 1–3 minute
gap, then the fallback assistant message at exactly t=300s from session start.
The Nth+1 LLM call — the one the operator most wants to see — is missing.

Triggering session of record: `46084b1a-d873-4e8a-92ae-4e76b0396348` (mika-arch,
2026-04-27).

## Root cause

`crates/mika-agent/src/agent.rs` had three call sites that wrapped their inner
async function in a single `tokio::time::timeout(Duration::from_secs(300), ...)`:

| Site               | Wrapped fn                | Mode |
|--------------------|---------------------------|------|
| `run_agent`        | `run_agent_inner`         | Conversation |
| `run_silent_agent` | `run_silent_inner`        | Heartbeat / callback / reflection / reminder / skill_run |
| `run_team_agent`   | `run_team_agent_inner`    | Team sub-agent |

When the 300s deadline fired while a `reqwest` HTTP request to an LLM provider
was in-flight, the inner future was dropped. `reqwest` cancelled the connection
at the socket layer. The DB write that records the `llm_calls` row happens
*after* the response parses, so a cancelled future never reached it.

The provider's per-request timeout (120s, configured at
`crates/mika-common/src/claude.rs:357-358`) was the correct cancellation
boundary for an in-flight HTTP request. The 300s "agent total timeout" should
have bounded *how many more steps we initiate*, not interrupted a running
provider call.

## Fix

Replace the three outer `tokio::time::timeout` wrappers with an `Instant`-based
deadline checked at four boundary points:

1. **Top of each step in `run_loop`.** Refuses to start the next step when the
   deadline has been reached. Any in-flight LLM call from the previous step has
   already completed (or transport-timed-out via the provider's 120s reqwest
   timeout) and persisted its `llm_calls` row by the time we re-enter this
   check.
2. **End of prelude work in each inner function.** Catches a pathological
   prelude (slow KG query, slow context fetch) that would itself blow the
   budget. Current prelude `.await` sites are sub-100ms — this gate is
   defensive against future slow paths.
3. **Before `attempt_continuation_turn`.** When `MAX_TOOL_STEPS = 20` is
   exceeded, the engine fires a continuation turn for a final summary. Skip
   continuation if `now + 60s > deadline`; otherwise clamp its inner timeout to
   `min(60s, deadline - now)`.
4. **Inside `attempt_continuation_turn`.** The continuation's own
   `tokio::time::timeout` had the same in-flight-cancel bug at smaller scale.
   Replaced with the deadline-aware clamp from (3) so in-flight LLM calls
   during continuation also persist their `llm_calls` row before being cut off.
   A `debug_assert!(deadline > Instant::now())` makes the gate-fires-first
   invariant from (3) machine-verifiable in test runs.

The provider's per-request 120s HTTP timeout is now the sole cancellation
mechanism for in-flight network work. Tool calls remain capped at 30s
(`TOOL_TIMEOUT_SECS`) by `dispatch_tool`'s own timeout.

`LoopResult` was promoted from a struct with a `max_steps_exceeded: bool` flag
to an enum with three variants — `Done`, `MaxStepsExceeded`, `DeadlineExceeded`
— so the compiler's match-exhaustiveness check enforces that all three outer
handlers (conversation, silent, team) handle every variant. `LoopResult` must
NOT carry `#[non_exhaustive]`; a wildcard `_ =>` arm could silently route a
future variant into the wrong fallback.

`tokio::time::Instant` is used (not `std::time::Instant`) so deadline checks
integrate with `tokio::time::pause()` virtual time in tests — see the eval
test at `crates/mika-agent/tests/eval/test_deadline_in_flight_llm_call.rs`.

## Worst-case turn duration

The new bound is `300s + 120s = 420s` (7 min) — when the final LLM call begins
at `t=299s` and runs until its 120s provider timeout. This is the explicit
trade-off: we accept up to 2 extra minutes of tail latency in exchange for
full diagnostic visibility on the call that was running when the deadline
fired. The 180s-tightened alternative (compute deadline as `300s - 120s` to
preserve the 5-minute user-facing contract) was **explicitly rejected**: it
adds reasoning complexity (which budget applies where?) without measurable
user benefit, since the tail-end LLM call is exactly the call the operator
most wants to see persisted.

## The `tokio::select!` guard

The fix's correctness depends on the deadline-check-at-iteration-top being
the only branch point inside `run_loop`. A future addition of `tokio::select!`
inside the loop body would shadow that guarantee — `select!` could resolve
on a different branch and drop in-flight DB writes (tool-call summaries,
message saves) mid-flight, re-introducing the silent-drop failure mode.

`scripts/check-loop-select.sh` greps the `run_loop` function body and fails CI
if `tokio::select!` appears inside it. Wired into `.github/workflows/ci.yml`
as the `loop-select-lint` job alongside `byte-slice-lint`.

If you have a legitimate need for `tokio::select!` inside `run_loop`, that's
a deliberate scope item — re-read this doc and the plan, then make a
considered change to the deadline enforcement model. Don't bypass the lint.

## Verification

The eval test `test_deadline_in_flight_llm_call.rs` uses
`tokio::time::pause()` + `MockResponse::Delayed` to simulate a slow provider
call under virtual time. Asserts:

1. The slow LLM call's `llm_calls` row IS persisted, even though it crossed
   the deadline. Pre-fix this would be 0.
2. The conversation-mode fallback message is emitted.
3. Exactly one `llm_calls` row was created — the deadline check at iteration
   top must have prevented a second LLM call.

Run with: `cargo test -p mika-agent --test eval test_deadline_in_flight_llm_call`.

## References

- `crates/mika-agent/src/agent.rs:36` — `AGENT_TOTAL_TIMEOUT_SECS` constant.
- `crates/mika-agent/src/agent.rs` — `LoopResult` enum, deadline check at the
  top of `run_loop`'s step iteration, `attempt_continuation_turn` deadline
  clamp + `debug_assert!`.
- `crates/mika-common/src/claude.rs:357-358` — per-provider 120s HTTP timeout.
- `crates/mika-common/src/llm/mock.rs` — `MockResponse::Delayed` variant uses
  `tokio::time::sleep`.
- `scripts/check-loop-select.sh` — CI lint guard.
- `.github/workflows/ci.yml` — `loop-select-lint` job.
- `docs/plans/2026-04-27-010-fix-agent-total-timeout-cancels-in-flight-llm-plan.md`
  — full plan with architect feedback.
- `docs/solutions/test-failures/lazy-pg-pool-blocks-30s-on-ci-2026-04-16.md:96`
  — prior mention of this diagnostic gap.
