---
title: Lazy Postgres pool in test AppState blocks ~30s on CI but RST-fails fast locally
date: 2026-04-16
problem_type: test_failure
track: bug
category: test-failures
module: mika-gateway
component: github webhook delivery tests
tags: [test-flake, ci, postgres, sqlx, timing, connect_lazy, AppState]
status: active
---

# Lazy Postgres pool in test AppState blocks ~30s on CI

## Problem

Two tests in `crates/mika-gateway/src/github.rs` (`test_deliver_semaphore_released_during_retry_sleep`, `test_deliver_abandoned_when_semaphore_full`) passed locally on every run but failed reliably on GitHub Actions CI. The production code under test was correct; the tests were correct in spirit. The asymmetry was environmental.

## Symptoms

- Local: 13/13 runs green, full crate suite in ~2s.
- CI: both tests panic with `permit should be released during retry sleep (waited 10.015359879s)`. Full crate suite takes ~62s on CI vs ~2s local — a 30× slowdown localized to a few tests.
- Initial guess: CI is just slow / racing the production retry sleep window. Wrong.

## What didn't work

1. **Polling helper with 1.4s budget.** Replaced bare `sleep(500ms)` assumption with `wait_for_permits()` that polls every 20ms up to 1.4s. Failed: CI's apparent latency exceeded the budget.
2. **Parameterizing the retry delay schedule.** Added `deliver_with_retry_inner` taking `&[Duration]`; tests injected `[60s]` so the released-permit window was huge regardless of CI speed. Still failed: `waited 10.015359879s` panic. The 10s budget never saw the permit released.

The "test polled 10 seconds and never observed release" was the smoking gun. The spawned task had not even completed the first HTTP attempt within 10 seconds. Something upstream of the HTTP call was eating the entire test budget.

## Root cause

`test_state_with_base_url()` builds the test `AppState` with:

```rust
let pool = sqlx::postgres::PgPoolOptions::new()
    .connect_lazy("postgres://fake:fake@localhost/fake")
    .expect("lazy pool");
```

The pool is **lazy** — it doesn't try to connect until first use. Production `deliver_with_retry` calls `resolve_github_container_url()`, which runs a real SQL query against this pool:

```rust
sqlx::query_as::<_, (uuid::Uuid, serde_json::Value)>(
    "SELECT customer_id, agent_mapping FROM github_repos WHERE repo_full_name = $1",
)
.bind(repo_name)
.fetch_optional(&state.pool)
.await
```

On a developer laptop (Linux): the connection to `localhost:5432` either succeeds (Postgres running) or **fails immediately with TCP RST** (port closed). sqlx returns the error in milliseconds; production code falls through to the `agent_base_url` fallback and the test proceeds.

On the GitHub Actions runner (Docker network, often lacks RST-fast on closed ports): the connection sits open until **sqlx's default `connect_timeout` (~30s)** expires. The spawned task is blocked inside `fetch_optional()` for the full 30s before the production code even reaches the HTTP attempt.

So the test wasn't racing the retry sleep at all — it was racing a 30-second silent block inside an "unrelated" production code path.

## Fix

Pass `repo_full_name = None` to `deliver_with_retry` in the timing-sensitive tests. The production code's `resolve_github_container_url` short-circuits the SQL query when no repo name is given and goes straight to the `agent_base_url` fallback (which the test sets to the mock server URL).

```rust
deliver_with_retry_inner(
    &state,
    "mika-dev",
    "test event",
    "delivery-sem",
    None,                        // ← was Some("org/repo")
    permit,
    &sem_clone,
    &TEST_DELAYS_OBSERVE_ONLY,
)
.await;
```

Other delivery tests in the same module (`test_deliver_success_on_first_attempt` etc.) keep `Some("org/repo")` because they don't have a polling budget — they just `await` the full call to completion and absorb whatever the SQL timeout costs.

## Why this works

- The Postgres lookup is skipped, so the spawned task gets to the HTTP attempt within milliseconds even on CI.
- The fallback path (`Some(ref base) = state.agent_base_url`) is well-exercised in production for single-tenant deployments — using it in tests is realistic.
- The test still exercises the production retry/release/abandon code paths fully — only the unrelated route-resolution step is skipped.

## Prevention

When writing tests against an `AppState` (or any shared state struct) that contains a connection pool:

- **Default to `connect_lazy` only if the test code path will demonstrably bypass the pool.** If a production code path called by the test will run a query, the pool is no longer "lazy from the test's perspective."
- **Use a short `connect_timeout` on test pools** if you must keep them. `PgPoolOptions::new().acquire_timeout(Duration::from_millis(100)).connect_lazy(...)` would have surfaced the same error on both local and CI in <100ms.
- **Treat localhost RST-fast as a portability hazard.** Tests that depend on "localhost:N is closed → fail fast" silently work on Linux desktops and silently fail on container CI. Either route around the dependency, or assert the behavior with a tight timeout.

## Related red flags this incident surfaced

- **`gh pr checks <pr>` returns non-zero exit when any check is failing.** Our `run_gh` tool wrapper treats non-zero exit as tool failure, which mika-dev then misclassified as "the tool errored" rather than "the checks are red." Worth a separate ticket.
- **Engine 5-min turn timeout has no diagnostic trail.** The agent emitted "I'm sorry, that took too long" twice; no LLM-call row, no error, no log pointer for the missing call. See related issue context in mika-platform memory `feedback_mika_dev_llm_fabricates_tool_errors.md` and the new mika#594 (ci_failure_handler).

## Sources

- PR #592 (mika#589, gateway inbound webhook retries) — the live example
- Failing CI runs: `gh run view 24490381168` and `gh run view 24509713058`
- Commits: `72628f9`, `4a12900`, `e6826b3` on branch `feat/589/gateway-retries-inbound-webhook-delivery`
- Related: mika#594 — `check_suite.completed(failure)` structural handler
