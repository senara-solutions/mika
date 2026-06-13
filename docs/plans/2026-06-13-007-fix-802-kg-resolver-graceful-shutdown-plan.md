---
ticket: mika#802
branch: fix/802/kg-resolver-graceful-shutdown
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/802
execution: code
---

# Plan: KG resolver + extractor graceful shutdown on SIGTERM (mika#802)

## Problem frame

Per-agent background tasks spawned by `server/mod.rs` — `kg::entity_resolver::SubjectEntityResolver` (resolver_tick, mika#906) and `kg::subject_extractor::SubjectExtractor` (extraction_tick, mika#1052) — do not respond to SIGTERM. During `make deploy`'s `stop → install → restart` window (~3-5s before supervise-daemon force-kills), these tasks continue writing rows under the OLD binary's config. The NEW binary then inherits stale rows and treats them as already-resolved, bypassing per-agent re-resolution.

Documented incident (2026-04-25 deploy of #795/#796):
- OLD binary's resolver wrote 279-424 rows/agent at 09:39:46-48Z under global `mika` docs_root_hash
- NEW binary at 09:39:50Z started with per-agent #778 routing
- Result: 3 odds-engine agents permanently `[DRIFT]`; required manual `mika kg purge --agent <name> --yes` to recover

Same failure mode applies to the extractor on its periodic tick.

## Directional decisions (per first-pass architect)

- **Grace period: 5 seconds.** Within typical OpenRC supervise-daemon defaults (10s SIGKILL grace). Implementer can tune via const.
- **Partial-write strategy: abandon.** On cancellation, in-progress LLM call results are dropped without writing. Half-resolved subjects stay `pending` for next startup. This is safer than "flush partial with sentinel outcome" because the sentinel outcome itself becomes a wire-protocol-style commitment.
- **Scope: both loops.** Resolver AND extractor both get the cancellation token. Asymmetric coverage would leak the same drift via the unfixed loop.

## Scope boundaries

- Pass a `tokio_util::sync::CancellationToken` into every per-agent tokio::spawn of resolver_tick + extraction_tick in `server/mod.rs`.
- Inside each tick loop, between iterations: `tokio::select!` on (a) next-tick deadline, (b) cancellation.
- Inside each LLM batch: check cancellation before initiating the next call; mid-call cancellation is bounded by `reqwest` request timeout (handler doesn't need to abort in-flight HTTP — the next-call check catches it within one call's duration).
- Wire SIGTERM handler in server main loop to trigger the token (mika-server uses Tokio's signal handlers; the existing graceful-shutdown path may already exist for axum).
- **Out of scope:** cancellation for non-KG background tasks (`checkpoint_task`, watchdog, reaper, parent-completer — those run for ms each cycle and don't have the drift failure mode); per-batch progress checkpointing (atomic-batch is the existing contract — abandon is sufficient); SIGTERM handling unification across all background tasks (separate concern).

## Implementation Units

### U1 — Add `CancellationToken` to resolver + extractor lifecycle

**Goal:** Both tick tasks accept a token; cooperative-cancel between iterations.

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (resolver_tick spawn site or its inner loop)
- Modify: `crates/mika-agent/src/kg/subject_extractor.rs` (extraction_tick spawn site or its inner loop)
- Modify: `crates/mika-agent/src/kg/resolver_tick.rs` if separate
- Modify: `Cargo.toml` — add `tokio-util` if not already a dep (likely already pulled in transitively)

**Approach:** The existing loops likely look like:

```rust
async fn tick_loop(...) {
    loop {
        sleep(interval).await;
        run_one_tick(...).await;
    }
}
```

Change to:

```rust
async fn tick_loop(..., cancel: CancellationToken) {
    loop {
        tokio::select! {
            _ = sleep(interval) => {}
            _ = cancel.cancelled() => {
                tracing::info!("kg tick cancelled (graceful shutdown)");
                return;
            }
        }
        // Check cancel before starting a batch — bounded latency
        if cancel.is_cancelled() {
            return;
        }
        run_one_tick(...).await;
    }
}
```

For mid-batch responsiveness, plumb the token into `run_one_tick` and check it between each LLM call inside the loop over pending entities:

```rust
async fn run_one_tick(..., cancel: &CancellationToken) {
    for entity in pending {
        if cancel.is_cancelled() { return; }  // abandon remaining
        // ... LLM call + DB write
    }
}
```

Per-iteration check means worst-case latency is one LLM call (~1-2s). With the 5s grace period, this is ample.

**Test scenarios:**
- **Token never cancelled:** loops run as before (verify existing tests still pass).
- **Token cancelled between batches:** loop exits cleanly without starting the next batch.
- **Token cancelled mid-batch:** current LLM call completes; subsequent entities are abandoned (not written).

**Verification:** unit tests using `CancellationToken::cancel()` against a mocked loop; integration test via test harness.

### U2 — Wire SIGTERM in server main loop

**Goal:** SIGTERM triggers the cancellation token; all per-agent KG tasks observe it.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` (the main loop / shutdown path)
- Modify: `crates/mika-agent/src/bin/mika-server.rs` if signal handling is there

**Approach:**

```rust
// Create the parent token once, before per-agent setup
let kg_shutdown_token = CancellationToken::new();

// Per-agent spawn site
let agent_token = kg_shutdown_token.child_token();
tokio::spawn(resolver_tick_loop(..., agent_token.clone()));
tokio::spawn(extractor_tick_loop(..., agent_token));

// Main loop: install SIGTERM handler that fires the parent token
tokio::spawn({
    let token = kg_shutdown_token.clone();
    async move {
        let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate()).expect("sigterm handler");
        sigterm.recv().await;
        tracing::info!("SIGTERM received; cancelling KG background tasks");
        token.cancel();
    }
});

// On graceful axum shutdown, await tasks complete (within grace window)
```

The 5s grace is enforced naturally by supervise-daemon: it sends SIGTERM, waits 10s, then SIGKILL. The KG tasks observe the cancel within one LLM call (~2s), cleanup completes, axum's existing shutdown finishes. We don't need an explicit deadline — supervisor's 10s window absorbs any tail latency.

**Test scenarios:**
- **SIGTERM received → token cancelled.** Manual test via test harness.
- **Tasks observe cancel within 5s.** Smoke test with `kill -TERM` + log monitoring.
- **Axum's existing shutdown still works.** No regression on HTTP server graceful exit.

**Verification:** integration smoke test on a local mika-server; check for the `kg tick cancelled` log line.

### U3 — Document the contract

**Goal:** `crates/mika-agent/CLAUDE.md` § Knowledge Graph documents the cancellation behavior.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Subject Extractor and § Entity Resolver sections

**Approach:** Add a paragraph under each:

> **Graceful shutdown (mika#802):** The tick loop accepts a `CancellationToken`. On SIGTERM (wired in server main), the parent token cancels all per-agent child tokens. The loop checks cancellation between iterations and inside per-batch entity loops. In-flight LLM calls complete (bounded by the per-request reqwest timeout, ~120s default but typically <2s); remaining pending work is abandoned (not written). Half-resolved subjects stay `pending` for the next startup. This prevents the OLD-binary-writes-under-stale-config drift documented in mika#802's incident.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 wires the token U1 plumbs through)
- U3 ships in same PR; last

## Patterns to follow (cross-cutting)

- `tokio_util::sync::CancellationToken` — standard tokio cancellation pattern, parent→child propagation
- `tokio::select!` on (interval, cancel) — common shutdown idiom
- Existing tick tasks in `kg::resolver_tick` / `kg::extractor_tick` — current loop shape

## Verification (top-level)

- `cargo test -p mika-agent kg::` passes (existing + new tests)
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Manual smoke: `kill -TERM <mika-server-pid>` mid-tick; verify `kg tick cancelled` log line; verify no DB rows written after the cancel timestamp; confirm no zombie task warnings.

## Risk / known unknowns

- **In-flight LLM call duration.** Each LLM call has a 120s `reqwest` timeout. In practice, KG calls return in 1-2s. Worst case (provider hang) means a stuck call holds shutdown for up to 120s — supervise-daemon's 10s SIGKILL backstop catches this. Within typical operation, cancellation observes within 2-3s.
- **Atomic-batch contract.** Existing code may rely on "a tick either fully completes or has not started" for idempotency. Abandon-mid-batch means a tick partially writes — subsequent restarts may see some entities resolved and others pending. This is already the existing contract (a batch can fail mid-way via LLM error); `resolve_pending` is idempotent — it re-attempts pending entities. Verified by reading existing code; no change to contract.
- **Test surface for cancellation.** Mocking `CancellationToken::cancelled()` requires an async test harness; standard tokio test patterns handle this.

## Out-of-scope (explicit)

- Cancellation for non-KG background tasks (`checkpoint_task`, watchdog, reaper, parent-completer) — those run for milliseconds per cycle and don't have the drift failure mode.
- Mid-LLM-call abort (drops the response). Bounded by reqwest timeout; not worth the complexity.
- Per-batch checkpoint files for resume-after-cancel — existing pending-detection handles this via DB state.
- Unified shutdown coordinator across all background tasks (separate architectural concern).
- Cancellation token threading through synchronous helpers — the loops are the natural cancellation boundary.
