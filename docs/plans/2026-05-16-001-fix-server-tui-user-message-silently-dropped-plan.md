---
title: Diagnose-then-fix silent drop of TUI user message after server restart
ticket: mika#850
type: fix
status: groomed
created: 2026-05-16
---

# Plan: Diagnose-then-fix silent drop of TUI user message after server restart (mika#850)

## TL;DR

The ticket reports a verified silent failure: row 23632 in `messages` exists for the user prompt `implement mika issue#848`, but neither `llm_calls` nor `server.log` records any agent-loop activity for that trace_id. The ticket's root-cause **hypothesis**, however, depends on a session-listener binding that does not exist in the current code. Before shipping the ticket's proposed fix shape (TUI restart detection, per-agent message watcher, observability metric), we must first **re-investigate forensically** to identify the actual mechanism. The plan structures the work as **diagnose → reproduce → fix → observability → ship**, where the shape of "fix" is gated on what diagnosis finds.

## Scope and contract

- **In scope:** Diagnostic re-investigation of the 2026-04-27 silent-drop incident; one concrete fix once root cause is established; an integration eval scenario that catches the same failure shape; a single observability event that surfaces "message persisted but no LLM call followed within N seconds."
- **Out of scope:**
  - `mika logs` CLI subcommand (companion ticket — separate work).
  - KG subject-extractor malformed-JSON retry loop on `openai/gpt-5-nano` (acknowledged in the ticket as a compounding factor, not causal — separate ticket if it surfaces independently).
  - General-purpose webhook/agent-loop resilience improvements beyond the specific class of failure observed here.

## Ticket-vs-code reality check (read first)

The ticket states: *"Session-listener binding does not survive server restart. When mika-server is restarted, in-memory state for the previous session's notify channel dies with the old process."*

Code inspection (handlers.rs:80–303 and surrounding modules) shows:

1. **`POST /message` always creates a fresh `session_id`** at `handlers.rs:751` via `uuid::Uuid::new_v4().to_string()`. The `MessageRequest` struct (server/types.rs:13–28) carries no `session_id` field. There is no path by which a client-supplied stale session_id reaches the server's persistence layer.
2. **No per-session in-memory listener / notify-channel / mpsc-sender registry exists.** The only broadcast channel in `AppState` is `a2a_broadcasters: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>` (server/state.rs:91), keyed by **task_id** (A2A SSE streaming only — orthogonal to message arrival).
3. **`mika chat --agent <name>` runs the agent loop locally in the TUI process** (chat.rs:265, `agent::run_agent(&AgentParams { … })`) against the shared SQLite file. It does NOT POST to mika-server. So mika-server's restart cannot directly silence a TUI-initiated turn.
4. **The ticket's forensic data point that worries us:** the message landed in session `63be052e` which was created at 21:48:44Z — two hours before the message. With handlers.rs:751 always generating a fresh UUID, that's not reachable from a `POST /message` flow. The most plausible explanation is that the TUI's local agent worker tokio task held `worker_session = 63be052e` since 21:48Z (chat.rs:215) and persisted into that same session via its in-process `run_agent` call — but then something prevented the LLM call from happening (or persisting an `llm_calls` row).

**This means: the symptom is real and verified, but the proposed fix shape (per-agent message subscription on the server) addresses a model that doesn't match production code.** We need a different theory before writing code.

## Step 1 — Forensic re-investigation (no code changes)

Goal: identify the actual code path that persisted message row 23632 and the actual reason `run_agent` didn't produce an `llm_calls` row.

### 1.1 Confirm the persistence path

```sql
-- Row 23632 metadata (was a metadata column written?)
SELECT id, session_id, agent_id, role, trace_id, metadata, internal, created_at
FROM messages WHERE id = 23632;

-- Session 63be052e channel_type and agent_id
SELECT id, agent_id, channel_type, created_at, ended_at
FROM sessions WHERE id = '63be052e-40b4-4a4a-b418-b3bac03df405';

-- Was there any session activity in the 5-minute window around 23:48Z?
SELECT id, role, created_at, length(content)
FROM messages WHERE session_id = '63be052e-40b4-4a4a-b418-b3bac03df405'
  AND created_at BETWEEN '2026-04-27T23:43:00Z' AND '2026-04-27T23:53:00Z'
ORDER BY created_at;

-- Is there a sibling row for the assistant turn that never completed?
SELECT id, role, trace_id, created_at
FROM messages WHERE session_id = '63be052e-40b4-4a4a-b418-b3bac03df405'
  AND created_at >= '2026-04-27T23:48:45Z'
ORDER BY created_at LIMIT 5;
```

The shape of these results tells us:
- `channel_type='cli'` → TUI-local run_agent path (chat.rs:265)
- `channel_type='telegram'` or `'github'` → server-side POST /message path (handlers.rs:80), which would contradict the "two hours old" observation
- A populated `metadata` column suggests `save_message_with_metadata` (called from inside `run_agent`); absent metadata + matching column shape suggests the team-path `save_message` (app.rs:844) which writes session_id `""` — but that doesn't match `63be052e` either

### 1.2 Read the right log sink

The crate-level CLAUDE.md (Log Sinks section) is explicit: **CLI invocations write to `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD`, not to `MIKA_SERVER_LOG_FILE`.** The ticket's investigation searched 2.5 GB of `/var/log/mika/server.log` — the correct sink for a CLI-initiated turn is the per-agent CLI log.

```bash
# Per-agent CLI log for mika-dev on both candidate dates
grep -F "d68e1285167a4fc0978bd655929e1cbe" ~/.mika/agents/mika-dev/logs/mika.log.2026-04-27
grep -F "d68e1285167a4fc0978bd655929e1cbe" ~/.mika/agents/mika-dev/logs/mika.log.2026-04-28
grep -F "63be052e-40b4-4a4a-b418-b3bac03df405" ~/.mika/agents/mika-dev/logs/mika.log.2026-04-{27,28}

# Same for the orchestrator agent if the message was typed in the orchestrator's TUI
grep -F "issue#848" ~/.mika/agents/*/logs/mika.log.2026-04-{27,28}
```

If the CLI log shows `run_agent` entry but no `llm_call` completion, that pinpoints a pre-LLM bail (context build, deadline pre-check, skill validation failure). If the CLI log shows nothing at all for that trace_id, the message was persisted by a path **outside** run_agent — narrows to (a) team-mode app.rs:844 save, or (b) a delegated path we have not yet considered.

### 1.3 Re-check the TUI worker liveness theory

```sql
-- Did the TUI process emit a panic/error event around 23:48Z?
SELECT created_at, event_type, payload
FROM audit_events
WHERE agent_id IN ('mika-dev', 'mika')
  AND created_at BETWEEN '2026-04-27T23:45:00Z' AND '2026-04-27T23:55:00Z'
ORDER BY created_at;
```

If the TUI's `tokio::spawn`'d agent worker (chat.rs:224) panicked, the channel send at app.rs:883 would succeed (mpsc unbounded), the receiver would never recv, run_agent would never fire, and the user would see their message in the chat history with no reply. That fits the observed symptom **exactly** and is the leading hypothesis after this code read.

### 1.4 Deliverable for Step 1

A short forensic note appended to this plan document (and referenced from the issue body's grooming summary) stating:
- Channel of session 63be052e (cli / telegram / github)
- Whether the per-agent CLI log shows `run_agent` activity for the trace_id
- Whether `audit_events` shows worker-task crash/panic in the window
- The remaining viable hypothesis (or "could not reproduce from forensics — escalate")

**Gate:** if Step 1 finds the root cause is unrelated to the ticket's stated hypothesis, **return to operator for confirmation** before proceeding to Step 3 — the fix shape will be materially different from what the ticket proposes. (Per `feedback_pre_dispatch_gate.md`: don't silently re-scope.)

## Step 2 — Reproduce the failure deterministically

Write a minimal reproduction harness that triggers the same shape (user message persisted, no `llm_calls` row, no error to user).

Two candidate harnesses depending on Step 1's outcome:

### 2.a If the path is `mika chat --agent <name>` (TUI-local run_agent)

Add a test under `crates/mika-cli/src/commands/chat.rs` `#[cfg(test)] mod tests` (or a new integration test under `crates/mika-cli/tests/`) that:
1. Spawns the agent worker with a mocked `LlmProvider` that panics on first call.
2. Sends an `AgentRequest::Message` over the mpsc channel.
3. Asserts: (a) the user message is persisted; (b) the worker task is no longer alive; (c) **a specific observable state** (currently nothing — the bug) records the worker death.

This test fails on `main` and passes after Step 3's fix. It is the regression test for the symptom class.

### 2.b If the path is server-mode `POST /message`

Add an integration test under `crates/mika-agent/tests/` that:
1. Spawns an Axum test server with an agent state.
2. Submits a `POST /message`.
3. Forcibly aborts the spawned tokio task at handlers.rs:288 (or injects a failure in `run_agent_for_message`).
4. Asserts the message persists, the response was 202, **and an observable event records the abort**.

### 2.c If neither — escalate

Step 1 found a path not covered by 2.a or 2.b. Surface to operator before continuing — the ticket scope may need re-definition.

## Step 3 — Fix (shape depends on Step 1)

### Branch A — TUI-local worker panic class (Step 1 finds the leading hypothesis)

Add a panic guard around the agent worker's tokio task in `crates/mika-cli/src/commands/chat.rs`:

1. Wrap the `tokio::spawn(async move { while let Some(request) = user_rx.recv().await { … } })` body in `tokio::task::JoinHandle` with `await`-on-completion error surfacing.
2. On `JoinHandle` resolving to `Err(JoinError)` (panic) or unexpected drop, send an `AgentResponse::WorkerCrashed { reason }` variant up to the TUI thread.
3. TUI displays a single line in the chat: `⚠ Agent worker crashed: <reason>. /restart to recover.` and accepts a `/restart` slash command that respawns the worker.
4. **Crucially:** if a message was sent to the channel right before the panic, the assistant response is missing — clarify to the user explicitly, do not let it go silent.

Touched files:
- `crates/mika-cli/src/commands/chat.rs` — JoinHandle supervision, restart message variant
- `crates/mika-cli/src/tui/app.rs` — new chat message style, status badge
- `crates/mika-cli/src/tui/commands/handlers.rs` — `/restart` slash command
- Test additions per Step 2.a

### Branch B — Server-mode POST /message tokio-spawn abort class (alternative)

Replace the fire-and-forget `tokio::spawn` at handlers.rs:288 with a `JoinHandle` retained in `AppState`, plus a watchdog task that:

1. Tracks in-flight per-request handles.
2. On JoinHandle resolving to `Err(JoinError)`, writes an `audit_events` row (`event_type = "message_processor_aborted"`) and emits a structured `error!` with the trace_id.
3. Optionally: writes a synthetic assistant message to the same session explaining the failure (only if outbound channel is available — never on `NoChannel`).

Touched files:
- `crates/mika-agent/src/server/handlers.rs` — JoinHandle supervision
- `crates/mika-agent/src/server/state.rs` — handle registry
- `crates/mika-agent/src/db.rs` — new audit event type constant (no schema change)
- Test additions per Step 2.b

### Why not "subscribe by agent_id" (ticket's suggestion 2)

The ticket suggests *"the agent-loop's message-arrival watcher should subscribe by `agent_id`, not by `session_id`"*. **There is no message-arrival watcher today.** The flow is direct: handle_message → tokio::spawn → run_agent. There is no pub/sub channel for the proposed change to apply to. Implementing such a subscription would be a net-new feature (introducing a global event bus for message arrivals), not a fix — and YAGNI/orthogonality both push back hard against it. We reject this suggestion explicitly.

### Why not "TUI detects server restart" (ticket's suggestion 1)

The TUI's local agent worker does not depend on mika-server for `mika chat` — the run_agent loop is in the TUI process. There is no server connection to "reconnect." If the operator runs `mika chat` against a remote server in a future deployment shape, restart-detection becomes relevant — but that's a different architecture from today's, and a different ticket.

## Step 4 — Observability event (always-on, applies to whichever Branch wins)

Add one new structured log event, emitted at the point where the new fix observes the failure:

- Event name: `agent_turn_silenced` (matches the "silently dropped" framing in the ticket title)
- Fields: `agent_id`, `session_id`, `trace_id`, `reason` (panic / abort / unknown), `last_message_id`, `seconds_since_message_persist`
- Level: `error!` (this is a user-visible failure)

This is the minimum observability surface. Do **not** introduce a new metrics subsystem or alerting plumbing for this — the audit event + error log is the existing pattern (see `crates/mika-agent/CLAUDE.md` § Observability), and we should not branch off into a third pattern for this one symptom.

## Step 5 — Ship

1. Run the full test suite (`cargo test`, `cargo clippy`, `cargo fmt --check`).
2. Run the eval harness for any newly added grounding/regression scenarios.
3. Open PR with a clear "Verification" section pointing at Step 1's forensic note as evidence that the root cause was confirmed, not assumed.
4. Manual smoke per `feedback_smoke_before_claiming_done.md`: run `mika chat`, force a worker panic via instrumentation, verify the new failure surface fires.

## Risks and open questions

1. **The ticket's hypothesis is materially wrong.** Operator must confirm we should proceed with diagnose-then-fix shape (this plan) rather than the ticket's prescribed listener-binding fix. Surface during second-pass architect review at the latest.
2. **Forensic data may be stale.** The incident is from 2026-04-27; today is 2026-05-16. SQLite WAL state and per-agent CLI logs may have rotated or been pruned. If Step 1 cannot reproduce forensically, we may need to instrument and wait for the next occurrence — which is a different shape of plan. Step 1 explicitly gates this.
3. **Branch A vs B choice depends on Step 1's outcome.** This plan keeps both pre-scoped so that we don't over-invest in one branch's implementation before evidence is in.
4. **Cross-cutting:** if the fix lands in `crates/mika-cli/`, it deploys via `make deploy` rebuilding the CLI binary — single-repo PR. No mika-cloud or mika-skills coordination needed.

## Sequence

1. Step 1 forensics (no code; 1–2 hours)
2. Operator gate (if root cause re-frame needed) — surface, wait for confirmation
3. Step 2 reproduction test (red)
4. Step 3 fix (test goes green)
5. Step 4 observability event wired
6. Step 5 ship

## Citations

- `crates/mika-agent/src/server/handlers.rs:751` — `let session_id = uuid::Uuid::new_v4().to_string();` (always-fresh session)
- `crates/mika-agent/src/server/types.rs:13–28` — `MessageRequest` carries no `session_id` field
- `crates/mika-agent/src/server/state.rs:91` — `a2a_broadcasters` (only broadcast registry; task-keyed, not session-keyed)
- `crates/mika-cli/src/commands/chat.rs:265` — `agent::run_agent(&AgentParams { … })` (TUI runs agent loop locally)
- `crates/mika-cli/src/commands/chat.rs:224` — `tokio::spawn(async move { while let Some(request) = user_rx.recv().await { … } })` (the worker task whose panic is the leading hypothesis)
- `crates/mika-agent/CLAUDE.md` § Log Sinks — server.log vs per-agent CLI log distinction (load-bearing for Step 1.2)
- `crates/mika-cli/src/tui/app.rs:844` — team-mode user-message save with `session_id=""` (excluded as the path for session 63be052e)
