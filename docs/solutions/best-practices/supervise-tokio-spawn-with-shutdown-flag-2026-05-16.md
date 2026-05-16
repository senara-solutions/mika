---
module: process-supervision
tags: [tokio, spawn, supervisor, joinhandle, mpsc, silent-drop, shutdown, atomic-flag]
problem_type: reliability
category: best-practices
related_issues: [1149, 1150, 959, 203]
---

# Supervise Long-Lived `tokio::spawn` Tasks with a Shutdown Flag (mika#1149)

## Context

`tokio::spawn` returns a `JoinHandle`, but binding it without observing it is the load-bearing mistake behind multiple silent-drop incidents in this workspace:

- **mika#1149** — TUI agent worker spawned at `crates/mika-cli/src/commands/chat.rs:224`. Panic inside the worker silently killed the receiver: the spawned future owned `user_rx`, so subsequent UI sends via `let _ = self.agent_tx.send(...)` kept succeeding because `mpsc::SendError` only fires when the receiver is *dropped*, not when the task holding it panics mid-iteration. The user prompt persisted to `messages` and the agent loop never ran.
- **mika#959** — server-side analogue: callback subprocess crashed without delivering, jamming the dispatch queue. Fixed with a `/proc/<pid>/stat` watchdog.
- **mika#203** — DB delivery queries filtered `status = 'completed'` only, hiding failed tasks — the write side couldn't distinguish a live consumer from a dead one.

The pattern recurs because **the channel-send API is the wrong signal for worker death**, and **JoinHandle is silent unless explicitly observed**.

## Guidance

For any `tokio::spawn` body that runs the main loop of a long-lived process (TUI worker, background callback handler, gateway router, etc.), wire a supervisor primitive that:

1. **Owns the worker's JoinHandle** and awaits it on a separate `tokio::spawn` (the supervisor task).
2. **Forwards both failure shapes** — `Err(JoinError)` for panics AND `Ok(())` for premature clean exit (the worker's loop returned because all senders were dropped or `break` was hit unexpectedly). Both are "worker died with in-flight work lost" from the operator's perspective.
3. **Reports failure via a callback**, not a hardcoded channel — keeps the primitive reusable across surfaces (TUI sees `AgentResponse::WorkerCrashed`; a server-side handler may want a structured log + metric + alert).
4. **Reads a shared `Arc<AtomicBool>` shutdown flag** before reporting failure. Operator-driven shutdown paths (Quit, agent switch, restart) set the flag *before* dropping senders or sending the termination request. The supervisor then exits silently when the worker resolves `Ok(())` for the expected reason.
5. **Clones the response sender** before moving the original into the worker closure. When the worker drops its sender on death, the supervisor's clone keeps the receiver open long enough to deliver the crash event.

The reusable primitive in mika lives at `crates/mika-cli/src/supervision.rs`. Its surface is small enough to copy or vendor into any crate that needs it:

```rust
pub struct WorkerFailure { pub reason: String }

pub async fn supervise<F>(
    worker_handle: JoinHandle<()>,
    shutdown_initiated: Arc<AtomicBool>,
    on_failure: F,
) where F: FnOnce(WorkerFailure)
{
    let join_result = worker_handle.await;
    if shutdown_initiated.load(Ordering::Acquire) { return; }
    let reason = match join_result {
        Err(e) if e.is_panic() => format!("worker panicked: {}", extract_panic_payload(e)),
        Err(e) => format!("worker task error: {e}"),
        Ok(()) => "worker exited before session closed".to_string(),
    };
    on_failure(WorkerFailure { reason });
}
```

## Why This Matters

Without supervision, a panic inside the worker is **completely invisible to the UI** because:

- The panic unwinds the spawned future's stack and drops the worker's locals — including its end of the mpsc channel.
- The UI's `agent_tx` clone is still alive (held by `App`), so `agent_tx.send(...)` returns `Ok` — the channel still has a sender, the message lands in the unbounded buffer.
- Nothing ever reads from the buffer (the worker is dead) and `agent_tx.send()` does not detect "no reader" — `SendError` only fires on receiver *drop*, not on consumer death.
- The user types, sees the spinner, never gets a response.

The supervisor pattern flips the signal: **the JoinHandle is the authoritative death signal**, not the channel. Once observed, both panic and premature `Ok(())` get the same user-visible treatment.

The `shutdown_initiated` flag prevents false-positive crash events on legitimate teardown paths. Store-Release on the setter side, Load-Acquire on the supervisor side; the store-before-drop ordering is the load-bearing invariant. If a future teardown path skips the store, the bug reintroduces — see mika#1150 finding F5 for the encapsulation follow-up (a `signal_shutdown()` method on the worker struct that owns the flag, making omission a compile-time gap).

## When to Apply

Apply this pattern when:

- A `tokio::spawn` runs a `while let` loop over an mpsc receiver and the consumer cannot observe its death via the channel API alone.
- A user-facing surface depends on the worker for liveness signal (TUI, dashboard, gateway).
- The session lifetime is bounded but longer than a single request (server worker pools, persistent agent loops).

**Don't bother** for:

- Fire-and-forget tasks that don't have a consumer waiting (e.g., a one-shot logging emit). The panic still unwinds; the consumer never knew it existed.
- Tasks already managed by a higher-level supervisor (the tokio runtime aborts everything on process exit anyway).
- Background tasks with their own liveness mechanism (mika#959 uses `/proc/<pid>/stat` for subprocesses — the right signal for that surface).

**Adjacent unguarded sites in this workspace** (filed in mika#1150's "Out of scope" section as a separate follow-up): `crates/mika-gateway/src/github.rs` has six `tokio::spawn` calls at lines 813, 2296, 2584, 2633, 2941, 2968 that may benefit from the same primitive if their failure modes can silent-drop. Evaluate per-site.

## Examples

### Before (mika#1149's pre-fix state — chat.rs:224)

```rust
let handle = tokio::spawn(async move {
    while let Some(req) = user_rx.recv().await {
        match req {
            AgentRequest::Message { text, .. } => {
                let result = agent::run_agent(...).await;
                // If run_agent panics, this whole closure unwinds.
                // user_rx is dropped, but agent_tx (held by App) keeps succeeding.
                let _ = agent_tx.send(response);
            }
            // ...
        }
    }
});
// handle is bound but never awaited. JoinError silently lost.
```

### After (mika#1149's fix — chat.rs:499 + supervision.rs)

```rust
// In spawn_agent_worker:
let supervisor_agent_tx = agent_tx.clone();      // clone BEFORE move
let handle = tokio::spawn(async move {
    while let Some(req) = user_rx.recv().await { /* ... */ }
});

let shutdown_initiated = Arc::new(AtomicBool::new(false));
let supervisor_shutdown = shutdown_initiated.clone();
let supervisor_handle = tokio::spawn(async move {
    mika_cli::supervision::supervise(handle, supervisor_shutdown, move |failure| {
        tracing::error!(
            target: "mika::otel",
            event = "agent_worker_silenced",
            reason = %failure.reason,
            "TUI agent worker entered failure state; pending prompt dropped"
        );
        let _ = supervisor_agent_tx.send(AgentResponse::WorkerCrashed {
            reason: failure.reason,
        });
    }).await;
});

// In every teardown path (Quit, switch, /restart), BEFORE dropping senders:
worker.shutdown_initiated.store(true, Ordering::Release);
let _ = app.agent_tx.send(AgentRequest::Quit);
```

### Test discipline

The supervisor primitive is testable without spinning up the agent loop — see `crates/mika-cli/tests/agent_worker_supervision.rs`. Three scenarios cover the contract:

```rust
#[tokio::test]
async fn worker_panic_surfaces_as_worker_crashed_reason() { /* ... */ }

#[tokio::test]
async fn worker_premature_clean_exit_surfaces_as_worker_crashed_reason() { /* ... */ }

#[tokio::test]
async fn operator_initiated_shutdown_silences_supervisor() { /* ... */ }
```

The wiring seam between the supervisor primitive and the surrounding lifecycle (clone ordering, store-before-drop, supervisor-handle replacement) is *not* covered by these tests — see mika#1150 finding T-03 for the wiring-test follow-up. The primitive contract is the load-bearing piece; the wiring is a separate test surface.

## Related

- [mika#959 callback watchdog (server-side analogue)](../959-callback-watchdog-stale-subprocess-detection.md) — same shutdown-silencing pattern (grace period instead of atomic flag), different liveness signal (`/proc/<pid>/stat`). The supervisor primitive here is the in-process equivalent.
- [mika#1066 TUI exit while busy](../1066-tui-exit-while-busy.md) — the `/restart` fast-path discipline this PR built on. Slash commands meant to recover from a stuck state must not gate on `AgentStatus::Idle`.
- mika#203 (failed callback tasks silently dropped) — same silent-failure class at the DB delivery layer. `AgentResponse::WorkerCrashed` is the type-level fix that doc points toward.
- mika#1150 — lifecycle-hardening follow-up cohort: agent-switch failure path, post-crash send guard, missing Quit on switch, restart/switch identity split, shutdown_initiated encapsulation. See [bug-class-fix-scope-vs-lifecycle-cohort](../workflow-issues/bug-class-fix-scope-vs-lifecycle-cohort-2026-05-16.md) for the framing rationale.
