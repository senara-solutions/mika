---
title: Supervise the TUI agent-worker task (mika#850 P0 finding extraction)
ticket: mika#1149
parent_ticket: mika#850
type: fix
status: groomed
created: 2026-05-16
revision: 3 (post second-pass mika-arch GROOMED — F3 extraction)
---

# Plan: Supervise the TUI agent-worker task (mika#1149, F3 extraction of mika#850)

> **Dispatch scope (mika#1149):** Implement the supervision fix only. The forensic verification phases below — Phase 0 pinning slices that reference mika#850's server-side paths (P0.1, P0.2, P0.3, P0.5, P0.6), the §F1 forensic re-investigation, and any §G acceptance gates that depend on forensic outcomes — are **out of scope for this PR per mika#1149 § Out of scope**. They remain mika#850's responsibility and will be addressed by the operator's separate forensic verification on `~/.mika/agents/<name>/logs/mika.log.2026-04-{27,28}`.
>
> **In scope for mika#1149:** §F2 (Supervise the TUI agent-worker task) — both the panic path and the premature-clean-exit (`Ok(())`) path; §F4 (TUI `/restart` slash command); the integration tests in §T (panic path + premature-clean-exit path). Use the issue body of mika#1149 as the implementation contract for failure-mode coverage, `AgentResponse::WorkerCrashed` variant shape, and acceptance criteria. If any §F2/§F4 step here conflicts with mika#1149's body, the body wins.

## TL;DR

mika#850 reports a verified silent failure: row 23632 in `messages` exists for the user prompt `implement mika issue#848`, but neither `llm_calls` nor `server.log` records any agent-loop activity for that trace_id. A pinned code read shows the ticket's stated mechanism (per-session listener binding that dies on server restart) does not exist in production code — and that `mika chat --agent <name>` runs the agent loop locally in the TUI process via an unsupervised `tokio::spawn` (a silent-panic bug class regardless of mika#850 specifically).

**Revised plan shape (post mika-arch first-pass ITERATE):** Make the fix unconditional — supervise the TUI agent worker's tokio task and surface its failure modes to the chat UI. The forensic re-investigation becomes a verification + spec-divergence reconciliation step (not a gate), with explicit time-boxing and fallback. If forensics shows mika#850's incident was caused by a *different* mechanism than the one Branch A defends, escalate before merging.

## Phase 0 — Pin (load-bearing source slices)

**Base commit:** `db3cdc1d156d` on `origin/main` (2026-05-16).

Six sites determine the plan's load-bearing claims. Verbatim slices:

### P0.1 — `POST /message` always creates a fresh `session_id`

`crates/mika-agent/src/server/handlers.rs:751`:

```rust
    let session_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) =
        a.db.create_session(&session_id, a.db.agent_id(), &req.channel)
            .await
    {
        warn!(error = %e, "failed to create session");
    }
```

There is no code path between the HTTP boundary and `create_session` that consults a client-supplied session_id.

### P0.2 — `MessageRequest` carries no `session_id` field

`crates/mika-agent/src/server/types.rs:11–28`:

```rust
/// Inbound message from the gateway.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MessageRequest {
    pub text: String,
    #[serde(default)]
    pub chat_id: Option<i64>,
    pub channel: String,
    pub request_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub images: Option<Vec<ImagePayload>>,
}
```

No `session_id` field. The Gateway protocol does not support session_id continuity for `POST /message`.

### P0.3 — Only broadcast registry is task-keyed (not session-keyed)

`crates/mika-agent/src/server/state.rs:90–91`:

```rust
    /// Active A2A task broadcasters for SSE streaming (keyed by task ID).
    pub a2a_broadcasters: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>,
```

No session-keyed sender registry exists anywhere in `AppState` or `AgentState`.

### P0.4 — TUI agent worker is unsupervised `tokio::spawn`

`crates/mika-cli/src/commands/chat.rs:215, 224`:

```rust
    let mut worker_session = session_id.clone();
    // … (worker state clones)
    let handle = tokio::spawn(async move {
        while let Some(request) = user_rx.recv().await {
            match request {
                AgentRequest::Message { text, images, thinking_budget } => {
                    // …
                    let result = agent::run_agent(&AgentParams { /* … */ }).await;
                    // …
                }
                // …
            }
        }
    });
```

The `handle: JoinHandle<()>` is bound but its result is never observed. A panic inside the loop body silently kills the receiver; subsequent UI sends at `tui/app.rs:883` succeed (mpsc unbounded) and are silently discarded.

### P0.5 — UI send path is fire-and-forget

`crates/mika-cli/src/tui/app.rs:883`:

```rust
        // Send to agent worker
        let _ = self.agent_tx.send(AgentRequest::Message {
            text,
            images,
            thinking_budget,
        });
        self.status = AgentStatus::Thinking;
```

The `let _ =` discards `SendError` (only fires when the receiver is *dropped* — not when the worker task panics mid-iteration with the receiver still owned by the spawned future).

### P0.6 — TUI run_agent invocation

`crates/mika-cli/src/commands/chat.rs:265–292` (call site for the local agent loop, channel `"cli"`, session_id from `worker_session`):

```rust
                    let result = agent::run_agent(&AgentParams {
                        db: &worker_db,
                        // …
                        channel_type: "cli",
                        session_id: &worker_session,
                        // …
                    })
                    .await;
```

This is the only direct `run_agent` call site in `mika-cli` (excluding the callback variant at chat.rs:377). The TUI does not POST messages to mika-spirit.

**P0 pin confirms** the plan's three pillars: (a) server has no session-listener registry; (b) TUI runs the agent locally with no supervision around the worker spawn; (c) message-arrival-via-restart hypothesis from the ticket has no underlying mechanism in this codebase.

## Scope and contract

- **In scope:**
  - Supervise the TUI agent worker tokio task; surface failures (panic, drop, await error) as a visible UI state.
  - Forensic re-investigation of mika#850's 2026-04-27 incident (time-boxed; verification, not gating).
  - Reconcile the issue body with the confirmed mechanism (mika-arch F3 — issue-as-versioned-contract).
  - One integration test that reproduces the worker-panic class deterministically.
  - One structured observability event emitted from the worker-supervision path.
- **Out of scope:**
  - `mika logs` CLI subcommand (companion ticket — separate work).
  - KG subject-extractor malformed-JSON retry loop (acknowledged in ticket as compounding, not causal).
  - Server-side `POST /message` supervision (Branch B from revision 1 of this plan, now reclassified — see § Branch B disposition).
  - Net-new global message-arrival event bus (explicitly rejected — see § Rejected directions).

## Step 1 — Supervise the TUI agent worker (UNCONDITIONAL fix)

The unsupervised `tokio::spawn` at `chat.rs:224` is a bug class regardless of mika#850's specific mechanism. Fixing it defends the silent-drop symptom (user message persisted, no response) for any failure of the worker task, including: panic in tool execution, panic in skill resolution, panic in `run_agent` itself, or a future code change that introduces a new panic-prone site.

### S1.1 — Retain the `JoinHandle`, observe its completion

Replace the current fire-and-forget pattern with a supervisor task that awaits the worker `JoinHandle` and forwards `JoinError` (panic) or unexpected completion (`Ok(())` from the `while let` loop terminating because `user_rx` closed) as a new `AgentResponse::WorkerCrashed { reason: String }` variant up the existing `agent_rx` channel.

The supervisor itself is a second `tokio::spawn`; its lifetime is bound to the chat command. The TUI's existing `tick()` loop consumes `AgentResponse` events and now also handles `WorkerCrashed` by:

1. Pushing a single `ChatRole::Command`-styled error line to the message list: `⚠ Agent worker crashed: <reason>. Use /restart to recover.`
2. Setting `self.status = AgentStatus::Idle` (clears the "Thinking" spinner left dangling by the lost send).
3. Recording the failure in `audit_events` via `db.log_audit_event("system", "agent_worker_crashed", …)` — this is the existing audit-write API; no schema change.

### S1.2 — `/restart` slash command

Add a `/restart` handler to `crates/mika-cli/src/tui/commands/handlers.rs` that:

1. Verifies the worker is in `WorkerCrashed` state (refuses on healthy worker; tells operator to use `/clear` for normal new-session flow).
2. Calls the existing `spawn_agent_worker` helper from chat.rs to spin up a fresh worker.
3. Rebinds the `agent_tx` channel on the `ChatApp` so subsequent sends route to the new worker.
4. Pushes a confirmation line: `Agent worker restarted.`

`/restart` does not replay the lost message; it restores the ability to send new ones. Replay would require either a "last-prompt" buffer (out of scope, adds state) or operator re-typing (preferred, no state).

### S1.3 — Touched files

- `crates/mika-cli/src/commands/chat.rs` — `JoinHandle` supervision (S1.1), `spawn_agent_worker` re-invocation path for `/restart`.
- `crates/mika-cli/src/tui/app.rs` — new `ChatMessage` variant / styling for crash line; `WorkerCrashed` handling in `tick()`; `agent_tx` rebind support.
- `crates/mika-cli/src/tui/commands/handlers.rs` — `/restart` slash command.
- (no changes outside `crates/mika-cli/` for Step 1)

## Step 2 — Reproduction test (red on main, green after S1)

Add `crates/mika-cli/tests/agent_worker_supervision.rs` (new integration test file under `crates/mika-cli/tests/`).

### S2.1 — Test approach

`MockLlmProvider` (`crates/mika-common/src/llm/mock.rs:86`) supports `MockResponse::Error(LlmError)` but not direct panic injection (NF2 from mika-arch). The reproduction does NOT need to panic the LLM provider — it needs to panic *the worker task* in a way that mirrors the production silent-drop shape.

Approach: write a thin integration test that spawns the agent worker with a panicking tool handler (a custom `Tool` impl whose `execute()` calls `panic!()`). The agent loop will:
1. Receive `AgentRequest::Message`.
2. Call `run_agent`.
3. `run_agent` schedules a tool call (mocked LLM response specifies the tool).
4. Tool execution panics.
5. `run_agent` returns `Err` OR the panic propagates up the `tokio::spawn` → `JoinHandle::Err(JoinError)`.

Both shapes (`run_agent` error vs. JoinError panic) must be observable through Step 1's supervision path. The test asserts: (a) a `WorkerCrashed` event arrives on `agent_rx` within 2 seconds; (b) the user message was persisted to DB (`messages` row exists); (c) no `llm_calls` row exists for that trace_id (matching the mika#850 forensic signature).

### S2.2 — Mock infrastructure check

If panicking-tool injection requires extending `mika-agent`'s `Tool` trait helpers, prefer adding a `#[cfg(test)]`-gated `PanicTool` rather than modifying production tool registration. Place it adjacent to the test file. No production-code change for test scaffolding.

### S2.3 — Test on main fails; on branch passes

The test must fail (timeout waiting for `WorkerCrashed`) against `db3cdc1d` (the pin). After S1 lands, the test passes within the 2-second window.

## Step 3 — Forensic verification (time-boxed; not a gate on S1)

**Purpose:** confirm that Step 1's supervision would have caught mika#850's specific incident. Decoupled from the fix itself.

### S3.1 — Time box

90 minutes maximum on the queries below. If forensic data has rotated past the window of useful inspection, mark inconclusive and proceed.

### S3.2 — Queries

```sql
-- Channel of session 63be052e
SELECT id, agent_id, channel_type, created_at, ended_at
FROM sessions WHERE id = '63be052e-40b4-4a4a-b418-b3bac03df405';

-- Metadata + trace_id for the dropped message
SELECT id, session_id, agent_id, role, trace_id, metadata, internal, created_at
FROM messages WHERE id = 23632;

-- Adjacent activity on the session
SELECT id, role, created_at, length(content)
FROM messages WHERE session_id = '63be052e-40b4-4a4a-b418-b3bac03df405'
  AND created_at BETWEEN '2026-04-27T23:43:00Z' AND '2026-04-27T23:53:00Z'
ORDER BY created_at;

-- audit_events around the incident window
SELECT created_at, event_type, payload FROM audit_events
WHERE agent_id IN ('mika-dev', 'mika')
  AND created_at BETWEEN '2026-04-27T23:45:00Z' AND '2026-04-27T23:55:00Z'
ORDER BY created_at;
```

### S3.3 — Logs (correct sink)

Per `crates/mika-agent/CLAUDE.md` § Log Sinks: CLI invocations write to `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD`, NOT to `MIKA_SPIRIT_LOG_FILE`. The ticket's 2.5 GB `server.log` search was the wrong sink for a CLI-channel session.

```bash
grep -F "d68e1285167a4fc0978bd655929e1cbe" \
  ~/.mika/agents/{mika-dev,mika}/logs/mika.log.2026-04-{27,28} 2>/dev/null
grep -F "63be052e-40b4-4a4a-b418-b3bac03df405" \
  ~/.mika/agents/{mika-dev,mika}/logs/mika.log.2026-04-{27,28} 2>/dev/null
grep -F "issue#848" \
  ~/.mika/agents/*/logs/mika.log.2026-04-{27,28} 2>/dev/null
```

### S3.4 — Three forensic outcomes

| Outcome | Meaning | Action |
|---------|---------|--------|
| **(a) channel_type='cli' + CLI log shows worker-task absence** | mika#850 was a TUI worker drop. S1's supervision covers it directly. | Reconcile issue body (S4). Proceed to merge. |
| **(b) channel_type='cli' + CLI log shows run_agent entry but no LLM call** | TUI worker reached run_agent but bailed pre-LLM (panic, deadline, context-build error). S1's supervision still catches the resulting JoinError or `Err` return. | Reconcile issue body (S4). Proceed to merge. |
| **(c) channel_type ≠ 'cli' OR forensic data rotated/inconclusive** | S1's mechanism may not be the same one that caused the incident. | Escalate to operator BEFORE merging. Document in S4 reconciliation that S1's fix is unconditional (defends a real bug class) but does not claim to have caught mika#850's specific incident. Operator decides merge disposition. |

## Step 4 — Reconcile the issue body (mika-arch F3 — issue-as-versioned-contract)

The plan reframes mika#850's hypothesis from "session-listener binding doesn't survive server restart" to "unsupervised TUI agent worker task; silent panic loses the in-flight prompt." Per the user_summary operating principle, the issue body must reflect the confirmed mechanism so that future readers of the issue see a consistent record.

### S4.1 — When (depending on S3 outcome)

- Outcomes **(a)** and **(b)** above: update the issue body to replace the "Root cause hypothesis" section with the confirmed mechanism. Add an edit-notice comment citing this plan's Phase 0 pins and the forensic findings from S3.2/S3.3. Closure annotation: cite both the original ticket framing and the corrected one.
- Outcome **(c)**: do NOT update the issue body unilaterally. Plan-on-branch retains the reframed analysis; issue body stays as-is pending operator's call. Add an edit-notice comment that says: "Phase 0 pin confirms the listener-binding mechanism is absent from current code (citations: handlers.rs:751, state.rs:91, types.rs:13–28). Forensic re-investigation [inconclusive | found a different channel]. Operator decision needed on whether to close as 'fixed by adjacent improvement' or hold open for further investigation."

### S4.2 — Audit trail format

```markdown
> **Hypothesis re-framed (mika-arch F3 reconciliation):**
> Original ticket hypothesis was "session-listener binding does not survive server restart."
> Phase 0 pinning against base commit db3cdc1d156d confirmed no such binding exists in current code.
> Confirmed mechanism: unsupervised tokio::spawn'd agent worker in mika-cli (chat.rs:224); on panic,
> mpsc-channel sends from UI succeed but the worker never runs run_agent.
> Forensic verification: <outcome a/b/c summary>.
> Fix: PR #<N> supervises the worker, surfaces failures as observable chat state, adds /restart.
```

## Step 5 — Observability event

One structured `error!` event, emitted from the supervisor task in S1.1 when the worker enters a failure state:

```rust
tracing::error!(
    target: "mika::otel",
    event = "agent_worker_silenced",
    agent_id = %agent_id,
    session_id = %session_id,
    reason = %reason,         // "panic" | "task_dropped" | "loop_exited"
    last_message_id = ?last_message_id,
    seconds_since_message_persist = ?elapsed_secs,
    "TUI agent worker entered failure state; pending prompt dropped"
);
```

NF3 reconciliation: this event fires from **the same `JoinHandle` error path** as S1.1's supervisor, not from a separate timer. `seconds_since_message_persist` is computed by recording the timestamp when `AgentRequest::Message` was sent and subtracting at crash time. No new timer infrastructure.

## Step 6 — Ship

1. `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
2. Run the integration test from S2 (specifically: `cargo test -p mika-cli --test agent_worker_supervision`).
3. Smoke-test (`feedback_smoke_before_claiming_done.md`): run `mika chat --agent mika-dev`, send a message, inject a panic via the test scaffolding (or comment out the supervision temporarily to confirm the symptom), verify both the symptom on `main` and the fix behavior on the branch.
4. Open PR; "Verification" section cites Phase 0 pin SHAs, S3 forensic outcome, and the test name.

## Branch B disposition

Revision 1 of this plan had a "Branch B — Server-mode POST /message tokio-spawn abort class" as an alternative fix shape. mika-arch NF1 correctly noted it was underspecified relative to Branch A. Disposition:

- Branch B is OUT OF SCOPE for mika#850. The Phase 0 pin confirms the TUI does not POST to the server; mika#850's incident channel cannot be Branch B's. Even forensic outcome (c) would not justify implementing Branch B — it would justify investigating which path was actually involved, which is a separate ticket if it surfaces.
- A future ticket may justify supervising `handlers.rs:288`'s spawned tasks for server-mode resilience. That ticket can cite this plan's reasoning. Not in scope here.

## Rejected directions (ticket suggestions; explicit rejection)

1. **"TUI side: detect server restart, start a fresh session."** The TUI's local agent worker does not communicate with mika-spirit for `mika chat`. There is no server connection to detect a restart of. Suggestion does not apply to current architecture.
2. **"Server side: subscribe by agent_id, not session_id."** Presupposes a per-session listener that does not exist (Phase 0 P0.3). Implementing it would be a net-new event-bus subsystem, not a fix. YAGNI + orthogonality reject this.
3. **"Observability: 'message-persisted-but-no-loop-tick-within-Ns' metric."** Replaced by Step 5's tighter `agent_worker_silenced` event, fired from the supervision path. The polling/timer shape implied by the ticket's framing is replaced by an event-driven shape that uses the same mechanism as the fix.

## Risks and open questions

1. **Forensic outcome (c) requires operator gate.** If S3 finds the incident channel was not `cli` (e.g., the forensic DB row pre-dates a code refactor that has since changed the session-creation contract), S1's fix is still a real bug-class fix but mika#850's specific incident remains unresolved. Operator must decide closure shape.
2. **Mock-tool panic harness may need a small `#[cfg(test)] PanicTool`.** Acknowledged in S2.2. Adds <30 LoC if needed; not a blocker.
3. **`/restart` interaction with active background tasks.** If the operator restarts the worker while background callback tasks are in-flight (counted in the footer `[N running]` badge), the new worker inherits the same agent state (DB, skills, identity) but loses any in-memory caches. Document this in the `/restart` confirmation message; do not implement in-memory state migration.
4. **No mika-cloud or mika-skills coordination needed.** Single-repo PR.

## Citations (Phase 0 source-of-truth)

All citations are pinned to base commit `db3cdc1d156d` on `origin/main`.

- `crates/mika-agent/src/server/handlers.rs:751` — `let session_id = uuid::Uuid::new_v4().to_string();`
- `crates/mika-agent/src/server/types.rs:11–28` — `MessageRequest` (no session_id field)
- `crates/mika-agent/src/server/state.rs:90–91` — `a2a_broadcasters` is the only broadcast registry; task-keyed
- `crates/mika-cli/src/commands/chat.rs:215, 224, 265–292` — worker spawn site; run_agent call site
- `crates/mika-cli/src/tui/app.rs:879–895` — UI send path; fire-and-forget `let _ = self.agent_tx.send(…)`
- `crates/mika-common/src/llm/mock.rs:86–158` — MockLlmProvider scope; supports Error injection, not panic injection
- `crates/mika-agent/CLAUDE.md` § Log Sinks — server.log vs. per-agent CLI log distinction

## Sequence

1. Phase 0 — already done (pins captured above).
2. Step 1 — supervise the worker (UNCONDITIONAL fix).
3. Step 2 — reproduction test (red → green).
4. Step 3 — forensic verification (90-min time box).
5. Step 4 — reconcile issue body per S3 outcome.
6. Step 5 — observability event (co-shipped with Step 1's supervisor).
7. Step 6 — ship.
