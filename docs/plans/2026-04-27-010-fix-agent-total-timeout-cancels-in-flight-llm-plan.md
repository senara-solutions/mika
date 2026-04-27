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

- `crates/mika-agent/src/agent.rs` — three outer-timeout removals; deadline plumbing into `run_loop`; structured warn on graceful exit.
- `crates/mika-common/src/llm/mock.rs` — add `MockResponse::Delayed { sleep_ms, inner }` variant so tests can simulate slow provider calls (test-utils-gated).
- `crates/mika-agent/tests/eval/` — new scenario verifying the deadline-during-llm-call invariant.

### Design

**1. Deadline plumbing.**

Each of the three outer functions computes a `deadline: Instant` and threads it through the call chain:

```rust
// In run_agent:
let deadline = Instant::now() + Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS);
let result = run_agent_inner(params, &trace_id, deadline).instrument(span).await;
match result { ... }  // No more tokio::time::timeout wrapper.
```

`run_agent_inner`, `run_silent_inner`, and `run_team_agent_inner` each accept a `deadline: Instant` argument. They pass it into `run_loop`.

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

`LoopResult` gains a `DeadlineExceeded` variant (it currently has `Done { ... }` and other terminal variants). The new variant carries enough state for callers to decide what to persist.

**3. Caller-side fallback handling.**

Each outer function maps `LoopResult::DeadlineExceeded` to its mode-appropriate fallback:

- **Conversation mode (`run_agent`):** save the existing `"I'm sorry, that took too long…"` message to DB exactly as today, return `AgentOutput { text: Some(fallback), ... }`. Same user-visible behavior, just no longer triggered by future-drop.
- **Silent mode (`run_silent_agent`):** today the timeout branch records a failed reflection run (`db.record_reflection_run("failed", 0, Some("Timed out"))`) for the `Reflection` trigger only. Preserve that exact behavior on `DeadlineExceeded`.
- **Team mode (`run_team_agent`):** today returns `Some("Agent timed out while processing team task.")`. Preserve that string return.

**4. In-flight LLM call protection.**

The deadline is checked *only* at the top of the step-loop iteration — not inside `attempt_continuation_turn`, not inside `dispatch_tool`, not inside the `llm.send_message(...)` await. This is the explicit design: any work already started gets to finish. The provider's 120s HTTP timeout is the cancellation mechanism for in-flight HTTP. Tools self-cap at 30s.

**5. Worst-case turn duration.**

`300s + 120s = 420s` (7 min) when the final LLM call begins at `t=299s`. Documented as the new bound. Acceptable in exchange for full diagnostic visibility.

**6. Test coverage.**

New eval-harness test in `crates/mika-agent/tests/eval/` (call it `deadline_in_flight_llm_call.rs`). Uses:
- `MockResponse::Delayed { sleep_ms: 350_000, inner: text_response("done") }` — simulates a slow provider call.
- `tokio::time::pause()` + `tokio::time::advance(...)` so the test runs in virtual time (no 5-min real wait).
- A short test-only deadline (e.g., 2s wall-clock) wired via a new `AgentParams.total_timeout: Option<Duration>` field — `None` defaults to `AGENT_TOTAL_TIMEOUT_SECS`, `Some(d)` overrides.

Test asserts:
1. The slow LLM call's `llm_calls` row is persisted (status either `success` or `error/timeout` — both are acceptable; the contract is *something is persisted*).
2. The fallback assistant message is saved.
3. The structured `agent deadline exceeded` warn is emitted.
4. The `dropped before persist` failure mode (no `llm_calls` row at all) does **not** occur.

Existing tests that depend on the old `tokio::time::timeout`-driven behavior: a grep finds none in `tests/`, so no regressions to update.

**7. Solution doc.**

Add `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` documenting:
- Failure mode (in-flight call cancelled, lost diagnostic trail).
- Fix (deadline checked between steps; provider timeout governs HTTP).
- Worst-case turn-duration bound (420s).
- Frontmatter: `module: mika-agent`, `tags: [timeout, observability, agent-loop]`, `problem_type: runtime-error`.

## Acceptance criteria

- [ ] No `tokio::time::timeout(Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS), ...)` wrapper around `run_agent_inner`, `run_silent_inner`, or `run_team_agent_inner`. Verified by grep in CI.
- [ ] An LLM call that crosses the 300s deadline persists an `llm_calls` row before the loop exits. Verified by the new eval test.
- [ ] Structured `warn!` with target `mika::otel`, fields `trace_id`, `steps_completed`, `mode`, message `agent deadline exceeded — exiting loop gracefully`. Replaces the current silent-drop log line.
- [ ] Same user-visible fallback message in conversation mode (`"I'm sorry, that took too long. Let me try a simpler approach next time."`).
- [ ] Same `record_reflection_run("failed", 0, Some("Timed out"))` side effect in silent reflection mode.
- [ ] Same `"Agent timed out while processing team task."` return in team mode.
- [ ] Solution doc at `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` with frontmatter and worst-case-bound section.
- [ ] No new clippy warnings; `cargo fmt --check` clean.
- [ ] `cargo test -p mika-agent` passes (existing 3253 tests + 1 new).

## Out of scope

- Changing `AGENT_TOTAL_TIMEOUT_SECS = 300`.
- Changing the per-provider HTTP timeout (120s in `claude.rs`, 120s in `openai.rs`).
- Tool-call timeout changes.
- Adding a runtime-configurable deadline for production (the new `AgentParams.total_timeout: Option<Duration>` field is test-utils-gated; production paths pass `None`).
- Auditing other `tokio::time::timeout` callsites (lines 437, 2047, 2070, 2097 — those are short-scoped continuation/utility timeouts, not the turn-level 300s budget).
- Per-step deadline checks within `attempt_continuation_turn` or tool dispatch — explicit design choice.

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| A misbehaving tool exceeds its own 30s timeout silently and the deadline never fires while it hangs. | The `tokio::time::timeout(TOOL_TIMEOUT_SECS, ...)` inside `dispatch_tool` is unaffected by this change — tools still self-cap. |
| A misbehaving provider exceeds 120s without timing out (e.g., never closes the connection). | The 120s `reqwest` client timeout is OS-level; if the provider never responds, the socket layer terminates. Worst case: 120s wait, then error. |
| The new `AgentParams.total_timeout` field gets passed `Some(short_duration)` from production code by accident. | Field is `Option<Duration>` defaulting to `None` (which yields the 300s constant). Test-utils-gated builder helpers in `EvalHarness` are the only setters in this PR. Reviewers will catch any production callsite that sets it. |
| Removing the outer timeout reveals a latent deadlock that was previously masked. | The continuation-turn timeout (60s, line 437) and continuation guards remain in place. If a deadlock exists, this change makes it observable rather than masking it as a 5-min hang. That's a feature, not a regression. |

## Files touched (estimate)

- `crates/mika-agent/src/agent.rs` — ~80 lines changed (three timeout removals + deadline plumbing + `LoopResult::DeadlineExceeded` variant + caller-side fallback handling).
- `crates/mika-common/src/llm/mock.rs` — ~30 lines added (`Delayed` variant + builder helper).
- `crates/mika-agent/tests/eval/deadline_in_flight_llm_call.rs` — ~100 lines new test.
- `mika/docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` — new file, ~80 lines.

Total: ~290 lines net.

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
