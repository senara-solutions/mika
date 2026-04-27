# Plan — fix: agent total timeout cancels in-flight LLM calls without diagnostic trail

**Ticket:** [mika#848](https://github.com/senara-solutions/solutions/mika/issues/848)
**Branch:** `fix/848/agent-total-timeout-cancels-in-flight-llm`
**Date:** 2026-04-27
**Author:** /mika-groom-ticket pre-architect draft

---

## Why

Three call sites in `crates/mika-agent/src/agent.rs` wrap their respective inner async function in a single `tokio::time::timeout(Duration::from_secs(300), ...)`:

| Site                     | Line   | Wrapped fn                     | Mode                  |
|--------------------------|--------|--------------------------------|-----------------------|
| `run_agent`              | 1414   | `run_agent_inner`              | Conversation          |
| `run_silent_agent`       | 2209   | `run_silent_inner`             | Heartbeat / callback / reflection / reminder / skill_run |
| `run_team_agent`         | 2671   | `run_team_agent_inner`         | Team sub-agent        |

When the 300s deadline fires while a `reqwest` HTTP request to an LLM provider is in-flight, the inner future is dropped. `reqwest` cancels the connection at the socket layer. Three things go wrong simultaneously:

1. **Lost response.** The provider may have been milliseconds from emitting `EndTurn`. The agent never receives it.
2. **No `llm_calls` row.** The DB write happens *after* the response parses. A cancelled future never reaches it. Latency, error message, stop reason, prompt variant — all gone.
3. **No diagnostic trail.** The agent emits the canned fallback `"I'm sorry, that took too long."` and the operator has no way to tell what was happening at t=300s.

Concrete evidence — session `46084b1a-d873-4e8a-92ae-4e76b0396348` (mika-arch, 2026-04-27):
- 4 successful `llm_calls` rows ending at `18:46:10Z` (calls 1–4, total ~141s).
- Then a 2m35s gap with no rows.
- Fallback assistant message saved at `18:48:45Z` — exactly t=300s from session start.

The 5th LLM call was in-flight when the agent timeout fired and was silently dropped.

This contradicts the per-provider HTTP timeout already configured in `crates/mika-common/src/claude.rs:357-358`:

```rust
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(120))
    .build()
```

That 120s is the correct cancellation boundary for an in-flight HTTP request. The 300s "agent total timeout" should bound *how many more steps we initiate*, not interrupt a running provider call. Once we let the provider's own timeout govern HTTP cancellation, the call either succeeds (and the response, `llm_calls` row, and stop reason are persisted) or it transport-times-out at t≤120s into the call (and the failure is persisted with `status='error'`, `error_message='timeout'`).

This pattern is already documented as a known diagnostic gap in `mika/docs/solutions/test-failures/lazy-pg-pool-blocks-30s-on-ci-2026-04-16.md:96`.

## What

Replace the three `tokio::time::timeout(...)` outer wrappers with an `Instant`-based deadline that is checked *between* steps in the shared `run_loop` (and at safe points in the silent/team loops, both of which call `run_loop` internally). The provider's own 120s HTTP timeout becomes the sole cancellation mechanism for in-flight network work. Tool calls already self-cap at 30s (`TOOL_TIMEOUT_SECS`) — the deadline will not interrupt them either; it just refuses to start the *next* step.

### Affected files

- `crates/mika-agent/src/agent.rs` — three outer-timeout removals; deadline plumbing into `run_loop`; deadline gates at `attempt_continuation_turn` entry and at prelude work; replace continuation's own inner `tokio::time::timeout` (line 437); structured warn on graceful exit.
- `crates/mika-common/src/llm/mock.rs` — add `MockResponse::Delayed { sleep_ms, inner }` variant using `tokio::time::sleep` (test-utils-gated).
- `crates/mika-agent/tests/eval/` — new scenario verifying the deadline-during-llm-call invariant.
- `scripts/check-loop-select.sh` (new) — pre-commit grep guard preventing `tokio::select!` inside `run_loop` body, wired into `ci.yml`.

### Design

**1. Deadline plumbing.**

Each of the three outer functions accepts (or computes) `deadline: Instant` and threads it through the call chain. Production callsites compute the deadline inline from the `AGENT_TOTAL_TIMEOUT_SECS` constant; tests construct their own `Instant` values directly. **No `Option<Duration>` knob is added to `AgentParams`** — the architect rejected that field per F4. The function signatures grow a `deadline: Instant` parameter; the test override path is "the test passes a different `Instant`," not "the test sets a knob."

```rust
// In run_agent:
let deadline = Instant::now() + Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS);
let result = run_agent_inner(params, &trace_id, deadline).instrument(span).await;
match result { ... }  // No more tokio::time::timeout wrapper.
```

`run_agent_inner`, `run_silent_inner`, and `run_team_agent_inner` each accept `deadline: Instant`. They pass it into `run_loop`. The `EvalHarness` test path constructs an `Instant` directly (e.g., `Instant::now() + Duration::from_millis(2000)`) and calls the functions on the test-utils-gated path.

**2. `run_loop` deadline check.**

Add a `deadline: Instant` parameter to `run_loop`. At the top of each loop iteration (`for step in 0..max_steps`, line 644 in `agent.rs`), check:

```rust
if Instant::now() >= deadline {
    warn!(
        target: "mika::otel",
        trace_id = %tool_ctx.trace_id,
        steps_completed = step,
        mode = mode.label(),
        "agent deadline exceeded — exiting loop gracefully"
    );
    return Ok(LoopResult::DeadlineExceeded {
        steps_completed: step,
        partial_summaries: all_tool_summaries,
        last_usage,
    });
}
```

`LoopResult` gains a `DeadlineExceeded` variant. Callers map it to mode-appropriate fallback (see step 3). **`LoopResult` must NOT carry `#[non_exhaustive]`** — the compiler's match-exhaustiveness check is the machine-enforcement surface that ensures all three outer handlers (conversation, silent, team) handle every variant. A `_ => { /* default */ }` arm could silently route a future variant into the wrong fallback.

**3. Caller-side fallback handling.**

Each outer function maps `LoopResult::DeadlineExceeded` to its mode-appropriate fallback, preserving today's user-visible behavior:

- **Conversation mode (`run_agent`):** save the existing `"I'm sorry, that took too long…"` message to DB, return `AgentOutput { text: Some(fallback), ... }`.
- **Silent mode (`run_silent_agent`):** preserve today's `db.record_reflection_run("failed", 0, Some("Timed out"))` for the `Reflection` trigger only.
- **Team mode (`run_team_agent`):** preserve today's `Some("Agent timed out while processing team task.")` return.

**4. Additional deadline gates (F3).**

Three additional check points beyond the step-loop top:

a. **Before `attempt_continuation_turn` (line 437).** When `MAX_TOOL_STEPS = 20` is exceeded, the engine fires a 60s continuation turn to coerce a text summary. If `Instant::now() + Duration::from_secs(CONTINUATION_TIMEOUT_SECS) > deadline`, skip continuation entirely and emit the structured fallback directly. Otherwise enter continuation but bound it by `min(60s, deadline - now)` rather than a fixed 60s. This addresses the 420s worst-case overrun where max-steps hits at t=299s and continuation runs another 60s.

b. **Before prelude work in `run_agent_inner` (and team/silent equivalents).** Prelude work — system-prompt assembly (`prompt::build_system_prompt`), context resolution (`resolve_contexts`), skill matching (`match_skills`), conversation-summary load — runs *before* the loop. A pathological prelude (huge KG query, slow context fetch) could itself blow the budget and leave the loop never reached. Add a deadline check immediately after prelude completes and before entering `run_loop`. If the prelude already exceeded the deadline, emit the fallback without entering the loop.

   **Granularity scope:** prelude is empirically fast (10–100ms for current `.await` sites). Threading `deadline` into every async helper (`resolve_contexts`, `match_skills`, etc.) is YAGNI scope creep. As an implementation pre-commit step, run `grep -n "\.await" crates/mika-agent/src/agent.rs` filtered to the prelude region of each outer-fn body — verify all calls hit known-fast paths. If a future prelude step has a documented slow path (e.g., a KG query that could block on a cold cache), thread `deadline` into it then. Tracked under § Out of scope.

c. **Replace continuation's own `tokio::time::timeout` (line 437) with deadline-aware wrapper.** The 60s timeout there has the same in-flight-cancel bug as the outer 300s — at smaller scale, but architecturally identical. Architect explicitly pulls this into scope. Replace with `tokio::time::timeout(min(60s, deadline - now), continuation_call)` so in-flight LLM calls during continuation also persist their `llm_calls` row before being cut off. The 60s ceiling stays — what changes is the deadline-clamp on top of it.

   At the clamp callsite, add `debug_assert!(deadline > Instant::now(), "deadline already passed at continuation entry — gate 4a should have prevented this")`. Stripped in release; fires in tests (including the new eval scenario's `tokio::time::pause()` path) if gate 4a's logic ever drifts. Makes the gate-fires-first invariant machine-verifiable.

**5. In-flight LLM call protection.**

The deadline is checked at four points: step-loop iteration top (`run_loop:644`), continuation entry (`attempt_continuation_turn` callsite), prelude completion, and continuation-call ceiling. It is **not** checked inside `dispatch_tool` or inside the `llm.send_message(...)` await — those have their own bounds (30s tool, 120s provider HTTP). Once an LLM HTTP request has started, it runs to completion or transport-timeout; the `llm_calls` row is always persisted.

**6. Worst-case turn duration.**

With the F3 gates, worst-case is now bounded at `300s + 120s = 420s` (7 min) — when the final LLM call begins at `t=299s` and runs until its 120s provider timeout. The 180s-tightened alternative (compute deadline as `300s - 120s` to preserve 5-min user-facing contract) is **explicitly rejected**: it adds reasoning complexity (which budget applies where?) without measurable user benefit, since the tail-end LLM call is exactly the call the operator most wants to see persisted in `llm_calls`. Documented as accepted bound with named rejection of the alternative.

**7. Test coverage.**

New eval-harness test in `crates/mika-agent/tests/eval/deadline_in_flight_llm_call.rs`. Uses:
- `MockResponse::Delayed { sleep_ms: 350_000, inner: text_response("done") }` — simulates a slow provider call. Implementation uses `tokio::time::sleep(Duration::from_millis(sleep_ms)).await` per F6 — **never `std::thread::sleep`**, which would block the runtime and defeat virtual-time control.
- `tokio::time::pause()` + `tokio::time::advance(...)` so the test runs in virtual time (no real wait).
- A short test-only deadline constructed inline (e.g., `Instant::now() + Duration::from_millis(2000)`), passed via the new `deadline: Instant` parameter on the agent entry function. No `AgentParams.total_timeout: Option<Duration>` field — that approach was rejected per F4.

Test asserts:
1. The slow LLM call's `llm_calls` row is persisted (status `success` or `error/timeout` — both acceptable; the contract is *something is persisted*).
2. The fallback assistant message is saved.
3. The graceful-exit code path runs (verified via the `DeadlineExceeded` variant returning from `run_loop` — F7 notes this is implicit, not a separate assertion).
4. The silent-drop failure mode (no `llm_calls` row at all) does **not** occur.

**8. Pre-commit guard (F5).**

Add `scripts/check-loop-select.sh` that greps `crates/mika-agent/src/agent.rs` for `tokio::select!` patterns inside `run_loop`'s body. The deadline-check-at-iteration-top guarantee assumes no `select!` shadows it (a `select!` inside the loop body could await on either the next step's work or some shutdown signal, dropping persistence DB writes mid-flight). Wire the script into `ci.yml` as a new job alongside `byte-slice-lint`. If a future change introduces `tokio::select!` inside the loop, CI fails with an explicit message pointing at this plan doc.

**9. Solution doc.**

Add `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` documenting:
- Failure mode (in-flight call cancelled, lost diagnostic trail).
- Fix (deadline checked at four boundary points; provider timeout governs HTTP).
- Worst-case turn-duration bound (420s) with rationale for rejecting the 180s tightening.
- The `tokio::select!` guard and why it matters.
- Frontmatter: `module: mika-agent`, `tags: [timeout, observability, agent-loop]`, `problem_type: runtime-error`.

## Acceptance criteria

- [ ] No `tokio::time::timeout(Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS), ...)` wrapper around `run_agent_inner`, `run_silent_inner`, or `run_team_agent_inner`. Verified by grep in CI.
- [ ] `run_agent`, `run_silent_agent`, `run_team_agent` accept (or compute and thread) a `deadline: Instant` argument. No `AgentParams.total_timeout: Option<Duration>` field exists.
- [ ] Deadline check at top of `run_loop` step iteration (`agent.rs:644`).
- [ ] Deadline check at entry to `attempt_continuation_turn`; continuation skipped if `now + 60s > deadline`. Continuation's own inner timeout is `min(60s, deadline - now)` rather than fixed 60s.
- [ ] Deadline check at end of prelude work in `run_agent_inner` and equivalents, before entering `run_loop`.
- [ ] An LLM call that crosses the 300s deadline persists an `llm_calls` row before the loop exits. Verified by the new eval test.
- [ ] An LLM call inside `attempt_continuation_turn` that crosses the deadline also persists an `llm_calls` row (smaller-scale variant of the same bug, fixed by F3c).
- [ ] Structured `warn!` with target `mika::otel`, fields `trace_id`, `steps_completed`, `mode`, message `agent deadline exceeded — exiting loop gracefully`.
- [ ] Same user-visible fallback message in conversation mode (`"I'm sorry, that took too long. Let me try a simpler approach next time."`).
- [ ] Same `record_reflection_run("failed", 0, Some("Timed out"))` side effect in silent reflection mode.
- [ ] Same `"Agent timed out while processing team task."` return in team mode.
- [ ] `MockResponse::Delayed` uses `tokio::time::sleep` (never `std::thread::sleep`).
- [ ] `scripts/check-loop-select.sh` exists, fails when `tokio::select!` appears inside `run_loop` body, and is wired into `ci.yml`.
- [ ] Solution doc at `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` with frontmatter, worst-case-bound section, and `tokio::select!`-guard rationale.
- [ ] No new clippy warnings; `cargo fmt --check` clean.
- [ ] `cargo test -p mika-agent` passes (existing 3253 tests + 1 new).

## Out of scope

- Changing `AGENT_TOTAL_TIMEOUT_SECS = 300`.
- Changing the per-provider HTTP timeout (120s in `claude.rs`, 120s in `openai.rs`).
- Tool-call timeout changes.
- Adding a runtime-configurable deadline for production (the new `AgentParams.total_timeout: Option<Duration>` field is test-utils-gated; production paths pass `None`).
- Auditing other `tokio::time::timeout` callsites (lines 437, 2047, 2070, 2097 — those are short-scoped continuation/utility timeouts, not the turn-level 300s budget).
- Per-step deadline checks within `dispatch_tool` — tools self-cap at 30s and the tool timeout is a different mechanism. (Note: `attempt_continuation_turn` *is* now in scope per F3, contrary to the original plan.)
- Threading `deadline` into prelude async helpers (`resolve_contexts`, `match_skills`, etc.) — current prelude `.await` sites are sub-100ms. Re-open this scope item if any future prelude step has a documented slow path.

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| A misbehaving tool exceeds its own 30s timeout silently and the deadline never fires while it hangs. | The `tokio::time::timeout(TOOL_TIMEOUT_SECS, ...)` inside `dispatch_tool` is unaffected by this change — tools still self-cap. |
| A misbehaving provider exceeds 120s without timing out (e.g., never closes the connection). | The 120s `reqwest` client timeout is OS-level; if the provider never responds, the socket layer terminates. Worst case: 120s wait, then error. |
| The new `AgentParams.total_timeout` field gets passed `Some(short_duration)` from production code by accident. | Field is `Option<Duration>` defaulting to `None` (which yields the 300s constant). Test-utils-gated builder helpers in `EvalHarness` are the only setters in this PR. Reviewers will catch any production callsite that sets it. |
| Removing the outer timeout reveals a latent deadlock that was previously masked. | The continuation-turn timeout (60s, line 437) and continuation guards remain in place. If a deadlock exists, this change makes it observable rather than masking it as a 5-min hang. That's a feature, not a regression. |

## Files touched (estimate)

- `crates/mika-agent/src/agent.rs` — ~120 lines changed (three timeout removals + deadline plumbing + `LoopResult::DeadlineExceeded` variant + caller-side fallback handling + F3 gates at continuation entry and prelude end + continuation inner-timeout deadline-clamp).
- `crates/mika-common/src/llm/mock.rs` — ~30 lines added (`Delayed` variant using `tokio::time::sleep` + builder helper).
- `crates/mika-agent/tests/eval/deadline_in_flight_llm_call.rs` — ~120 lines new test (now covers continuation in-flight as well).
- `scripts/check-loop-select.sh` — new file, ~20 lines.
- `.github/workflows/ci.yml` — ~10 lines added for the loop-select-lint job.
- `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` — new file, ~100 lines.

Total: ~400 lines net.

## Verification

Local before opening PR:

```bash
cd mika
cargo build -p mika-agent
cargo test -p mika-agent --lib agent
cargo test -p mika-agent --test eval deadline_in_flight_llm_call
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt -- --check
```

CI gates (existing): `ci.yml` runs full `cargo test` + clippy + fmt + byte-slice-lint. No new gates required.

Manual smoke test (optional, post-merge): trigger a long mika-arch session with a slow provider, confirm fallback message appears and `llm_calls` shows a final row (success or transport-timeout, not silent gap).

## References

- `crates/mika-agent/src/agent.rs:36` — `AGENT_TOTAL_TIMEOUT_SECS` constant.
- `crates/mika-agent/src/agent.rs:1414, :2209, :2671` — three `tokio::time::timeout` callsites to remove.
- `crates/mika-agent/src/agent.rs:644` — `for step in 0..max_steps` — deadline-check insertion point.
- `crates/mika-common/src/claude.rs:357-358` — per-provider 120s HTTP timeout.
- `crates/mika-common/src/llm/openai.rs:153` — per-provider 120s HTTP timeout (OpenAI-compatible adapter).
- `mika/docs/solutions/test-failures/lazy-pg-pool-blocks-30s-on-ci-2026-04-16.md:96` — prior mention of this diagnostic gap.
- Triggering session: `46084b1a-d873-4e8a-92ae-4e76b0396348` (mika-arch, 2026-04-27).
