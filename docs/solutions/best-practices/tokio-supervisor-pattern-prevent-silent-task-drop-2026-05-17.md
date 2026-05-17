---
title: Supervise tokio::spawn'd workers — prevent silent task drop and second-order channel-of-dead-receiver
module: mika-cli
date: 2026-05-17
problem_type: best_practice
component: tooling
severity: high
related_components:
  - mika-agent
  - assistant
tags:
  - tokio
  - supervisor
  - jointhandle
  - panic
  - mpsc
  - async
  - tui
  - agent-loop
applies_when: >
  Spawning a long-running tokio task whose failure (panic or unexpected exit)
  must be visible to the user — and whose work is fed by an mpsc channel that
  the rest of the app keeps using after the worker is gone.
---

## Context

mika-cli's TUI ran the agent loop inside an unsupervised `tokio::spawn`:

```rust
let (user_tx, mut user_rx) = mpsc::unbounded_channel::<AgentRequest>();
let (agent_tx, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

let handle = tokio::spawn(async move {
    while let Some(req) = user_rx.recv().await {
        // ... process, send response via agent_tx ...
    }
});

// `handle: JoinHandle<()>` bound but never observed.
```

The TUI submitted messages by `let _ = self.agent_tx.send(AgentRequest::Message { ... })` and treated the SendError as impossible. Two failure modes hid behind that pattern:

1. **First-order silent drop (mika#850 root):** if the worker task panicked inside `run_agent`, the closure was dropped, `agent_tx` was dropped, and `agent_rx.try_recv()` started returning `Disconnected` — but only the "status == Thinking" branch of `tick_agent_mode` rendered anything, and only that branch existed. Outside that narrow window the panic was invisible: the user typed, saw their message persisted, saw a Thinking spinner, and never got a reply. `messages` row existed in SQLite; `llm_calls` row did not.

2. **Second-order silent drop (caught in ce:review adv-1):** even after surfacing a crash banner, the next keystroke recreated the bug. `App::send_message` ran `let _ = self.agent_tx.send(...)`. On a dead receiver the `SendError` is still swallowed, the user message is still pushed to chat history, and the Thinking spinner is set. The fix only surfaces the *first* crash; every subsequent send re-enters the silent-drop UX until the operator types `/restart`.

A third habit made teardown unsafe: `worker_abort.abort()` was used aggressively on every teardown path. Abort cancels the worker at its next `.await`, which means the worker's post-loop `mcp.shutdown().await` block was skipped on `/agent` switch and app-exit — leaking MCP child processes and HTTP-transport session state.

## Guidance

When spawning a long-running worker whose failure must be observable, apply three patterns together. Each defends a distinct failure class; any one alone leaves a hole.

### 1. Supervisor task — observe the JoinHandle

Spawn a sibling task whose only job is to await the worker's `JoinHandle` and forward the outcome on the existing response channel. Pre-clone a `Sender` for the supervisor; the worker keeps the original.

```rust
let supervisor_tx = agent_tx.clone();        // supervisor's clone
let quit_received = Arc::new(AtomicBool::new(false));
let worker_quit_flag = quit_received.clone();

let worker_handle = tokio::spawn(async move {
    while let Some(req) = user_rx.recv().await {
        // ...
        match req {
            AgentRequest::Quit => {
                worker_quit_flag.store(true, Ordering::Release);
                break;
            }
            // ...
        }
    }
});
let worker_abort = worker_handle.abort_handle();   // for intentional teardown

let supervisor_handle = tokio::spawn(async move {
    let outcome = worker_handle.await;
    let reason = match outcome {
        Ok(()) => {
            if quit_received.load(Ordering::Acquire) {
                return;                            // clean Quit — no notification
            }
            "worker_loop_exited_without_quit".into()
        }
        Err(e) if e.is_cancelled() => return,      // intentional abort — silent
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            let msg = payload
                .downcast_ref::<&str>().map(|s| (*s).into())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic>".into());
            format!("panic: {msg}")
        }
        Err(e) => format!("join_error: {e}"),
    };
    tracing::error!(
        target: "mika::otel",
        event = "agent_worker_silenced",
        reason = %reason,
        "TUI agent worker entered failure state"
    );
    let _ = supervisor_tx.send(AgentResponse::WorkerCrashed { reason });
});
```

Key invariants:

- **Channel typed as an enum, not a struct.** `AgentResponse::Reply { ... } | AgentResponse::WorkerCrashed { reason }` lets a single channel preserve send-order semantics. A sibling channel for "control" events forces the consumer to poll two receivers per tick and can deliver crash-after-reply out of order.
- **`quit_received: Arc<AtomicBool>` distinguishes clean shutdown from loop bug.** Without this, every `Ok(())` exit (including normal `/agent` switch and `/exit`) fires a spurious crash banner. `Release` on store + `Acquire` on load is sufficient; `JoinHandle::await` also establishes a happens-before edge, so the ordering is belt-and-suspenders.
- **`AbortHandle`, not the JoinHandle, for intentional cancel.** Awaiting the JoinHandle is the supervisor's job. The run loop needs a separate `AbortHandle` so it can request cancellation without moving the JoinHandle out from under the supervisor.

### 2. Guard the producer against the dead receiver

The supervisor surfaces the *first* crash. The second-order silent drop lives in `let _ = sender.send(...)` patterns scattered through the app — every one of them swallows `SendError` and lets the user think their input landed.

Add a single state field on the App and gate every send-from-user path on it:

```rust
pub struct App {
    // ...
    pub worker_crashed: Option<String>,
}

pub fn send_message(&mut self, text: String) {
    if self.worker_crashed.is_some() {
        self.messages.push(ChatMessage::system(
            "Agent worker has crashed. Run /restart before sending more messages."
        ));
        return;
    }
    // ... normal path: let _ = self.agent_tx.send(...);
}
```

The check belongs at the *call site*, not inside `send`. The call site has the context to render a useful failure message; `send` only has a generic `SendError`.

Pair the guard with a recovery affordance — for mika-cli, a `/restart` slash command that respawns the worker via the same `spawn_agent_worker` helper and clears `worker_crashed = None`. Reuse the session_id so conversation context survives; do not replay the lost in-flight prompt (operator re-types).

### 3. Teardown — Quit-first, abort fallback (preserve graceful shutdown)

`worker_abort.abort()` is a sledgehammer: the worker is cancelled at its next `.await`, dropping any work in flight. Use it only when graceful shutdown isn't an option (the worker is presumed crashed, or has been given a clean signal and hasn't honored it).

For routine teardown (`/agent` switch, app-exit) signal first, then drain with timeout, abort only as fallback:

```rust
// handle_switch already sent AgentRequest::Quit on the user_tx
worker.poller_handle.abort();
let supervisor = std::mem::replace(&mut worker.handle, tokio::spawn(async {}));
if tokio::time::timeout(Duration::from_secs(2), supervisor)
    .await
    .is_err()
{
    tracing::warn!(event = "supervisor_drain_timeout", site = "agent_switch", "aborting");
    worker.worker_abort.abort();
}
```

This shape preserves the worker's post-loop cleanup (`mcp.shutdown().await`, etc.) on the common case and falls back to abort only when the worker is wedged.

For `/restart` (worker is presumed crashed, no point sending Quit) skip directly to abort:

```rust
worker.poller_handle.abort();
worker.worker_abort.abort();
let _ = tokio::time::timeout(Duration::from_secs(2), worker.handle).await;
```

### 4. Don't block the tick loop on async I/O

The `WorkerCrashed` handler is tempting to enrich with an audit row, a metric, etc. — but the tick loop is the input path. An `await` here is back-pressure on the user's keystrokes.

Fire-and-forget the side-effect:

```rust
Ok(AgentResponse::WorkerCrashed { reason }) => {
    self.messages.push(ChatMessage::system(format!("⚠ Agent worker crashed: {reason}.")));
    self.status = AgentStatus::Idle;
    self.needs_redraw = true;

    let db = self.db.clone();
    let session = self.session_id.clone();
    let reason_audit = reason.clone();
    tokio::spawn(async move {
        if let Err(e) = db.log_audit_event(&session, "system", "agent_worker_crashed", None, Some(&reason_audit), None, None).await {
            tracing::warn!(error = %e, "failed to log agent_worker_crashed audit event");
        }
    });

    self.worker_crashed = Some(reason);
}
```

This matters most precisely when the worker crashed mid-DB-closure — the DB is the most likely source of back-pressure on this exact code path.

## Why This Matters

`tokio::spawn` returns a `JoinHandle` whose `Drop` impl does **not** cancel the task and does **not** propagate panics. Binding the handle to `_handle: JoinHandle<()>` and never reading it is the canonical recipe for silent task drop. The compiler will not warn; clippy will not warn (the variable is "used"). The only way to detect that a long-running task has died is to read its handle, or to design the channel topology so the death is observable downstream.

The second-order trap is more subtle: `mpsc::UnboundedSender::send` succeeds (queues into channel) until the receiver is dropped, at which point it returns `SendError(value)`. Most code wraps it `let _ = tx.send(...);` because the "happy path" never fails. After the worker dies the receiver is dropped, every send fails, and every `let _` swallows the failure. The fix isn't to plumb `Result` everywhere — it's to track the worker's liveness as App state and gate the call sites that need to surface failure.

The teardown lesson is orthogonal but commonly co-discovered: aggressive `abort()` cancels graceful shutdown work that you may not have noticed the worker was doing. Treat abort as a fallback after a Quit-then-timeout, not as the default path.

## When to Apply

- Any long-running `tokio::spawn` whose failure must be user-visible (TUI workers, server background tasks that own a user-facing channel, dispatcher loops).
- Any `mpsc` channel where the receiver is owned by a spawned task that can die independently of its senders.
- Any teardown path that aborts a task running cleanup code at the end of its loop (MCP shutdown, DB transaction commit, file flush).

Not needed for short-lived spawns (tool calls, one-shot side effects) where the parent task can `.await` the handle directly and propagate the result.

## Examples

The full pattern landed in mika#850 (PR: TBD). Key sites:

- `crates/mika-cli/src/commands/chat.rs` — `spawn_worker_supervisor()` extracted as a testable helper; four `#[tokio::test]` cases cover panic / loop-exit-without-quit / clean-quit / cancelled paths.
- `crates/mika-cli/src/tui/app.rs` — `AgentResponse::{Reply, WorkerCrashed}`, `worker_crashed: Option<String>`, `send_message` guard.
- `crates/mika-cli/src/tui/commands/handlers.rs` — `/restart` handler.

## Prevention

If your repo has a lot of unobserved `JoinHandle`s, a static check is feasible: forbid `tokio::spawn` outside of `pub(crate) fn spawn_*_supervised` helpers, and require those helpers to either `await` the handle, route it through a known supervisor, or wire it into `tokio_util::task::TaskTracker`. mika hasn't gone this far yet — flagging here as the next-level mitigation if a second instance of this bug class shows up.
