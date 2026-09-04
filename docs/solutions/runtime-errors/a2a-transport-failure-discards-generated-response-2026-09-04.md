---
module: mika-a2a, mika-cli, mika-agent
tags: [a2a, reqwest, timeout, transport, recovery, context-id, error-messages, mika-ask]
problem_type: bug
category: runtime-errors
date: 2026-09-04
---

# A transport failure threw away an answer the server had already produced (mika#2036)

## Problem

`mika ask` reported `connection error: error sending request` for exchanges the
server had **fully generated**. The work was done, billed to the provider,
persisted to SQLite — and then discarded, because the socket carrying it home
had closed. The caller was told the server was unreachable.

Two independent measurements, three weeks apart, from opposite seats:

| date | seat | what was observed |
|---|---|---|
| 2026-08-29 | Vincent's CLI | 3 architect briefs; 2 lost. Both responses recovered **by hand** from `/var/log/mika/server.log`. The `--session-id` of the second pass travelled in the lost envelope, so the follow-up ran without it. |
| 2026-09-04 | orchestrator, scripted | 8 attempts on `mika ask --agent mika-qa --json`, growing backoff over ~20 min. Zero-byte capture every time, **exit code 0** every time. mika-qa was not mute — it returned verdicts on three other PRs in the same window. |

Payload size is not the discriminant (10 492 B succeeded, 8 345 B failed). The
only long generation delivered took **114 s**; the lost ones were longer.

## Root cause — three separate defects on one path

1. **No timeout policy.** `crates/mika-a2a/src/client.rs` built its client with
   `reqwest::Client::new()`, which applies *no request timeout at all*. The
   behaviour was whatever the OS and the peer happened to do — not a decision.
2. **An error that lied about itself.** `remote_ask.rs` flattened every
   `A2aError::ClientError` into `connection error: {e}`. "I could not reach the
   server", "I waited N seconds and gave up", and "the socket died after the
   request landed" all rendered the same sentence. That sentence read as *server
   down* while `/health` answered in **0.5 ms**.
3. **No way to ask what became of the work.** The task id is minted server-side
   (`Uuid::new_v4`, `server/a2a.rs`) and travels back only in the envelope that
   was lost, so the caller had no handle to look anything up with.

The second measurement added the sharpest framing: **a caller cannot distinguish
"busy, retry" from "your answer exists and was lost"** — and only the second
justifies an escalation. A timeout bounds the waiting; it still does not say why
nothing came.

## The fact that makes recovery possible

`handle_message_send` persists the finished task — state `completed`, messages
inserted — **before** it serializes the HTTP response (`server/a2a.rs`). So a
response lost in transit always names work that is already on disk. This was
verified by reading the handler, not assumed from the symptom.

## Fix

- **`TransportFailure`** (`mika-a2a/src/error.rs`) classifies a `reqwest` failure
  into unreachable / timed out / HTTP status / unreadable / interrupted. Its
  load-bearing question is `request_was_sent()`: **only an unreachable server
  proves no work exists.** `is_connect()` is tested before `is_timeout()` because
  a connect timeout reports both and means the server was never reached.
- **`DEFAULT_TIMEOUT = 300 s`**, a named constant. Measured, not generous: 2.6x
  the longest delivered generation (114 s). `A2aClient::new` keeps its signature;
  `with_timeout` overrides; `RECOVERY_TIMEOUT` (30 s) bounds the recovery read,
  which is a database read and must not inherit a generation-sized budget.
- **Correlation on `context_id`.** The caller mints one *before* sending, so it
  survives the send failing. The server already persisted it on `a2a_task_map`;
  `tasks/get` now resolves it — **after** the task-id lookup misses, so a real
  task id can never be shadowed.
- **Five outcomes, five sentences.** `Recovered` returns the answer.
  `StillRunning` / `NoTask` say *retry*, for different reasons. `Ended` says the
  task will produce nothing. `Unavailable` says an answer may exist and names the
  `context_id` to query.

## The reusable lesson

**A report that collapses distinct situations into one sentence is worse than no
report** — it is confidently wrong, and it is acted on. The cost here was not the
lost bytes; it was two hours chasing a healthy server, and a scripted caller
concluding an agent was mute while it was demonstrably answering elsewhere.

The generalisation: when an operation fails, the caller's next question is never
only *what broke* — it is **what became of my work**. An error that answers the
first and stays silent on the second forces the reader to guess, and the guess is
usually "it's gone."

## Anti-vacuity, and why it was needed here

The founding defect was a **shared** message, not a missing one. A test that
asserts each message is non-empty would have passed before the fix. So the tests
compare every pair of rendered outcomes and fail the moment two collapse again
(`transport_failure_descriptions_are_distinct`,
`each_outcome_reads_differently_from_every_other`).

Two probes could not run before the fix at all, which is the strongest form of
proof available here:

- `a_silent_server_is_abandoned_on_the_clients_budget` — with no timeout, it
  never terminated.
- `a_generated_response_survives_a_dropped_socket` — a raw `TcpListener` reads
  the `message/send` and hangs up without answering, then serves `tasks/get`. It
  also asserts the recovery queried **the context the caller itself sent**, so it
  cannot pass against a server that answers any id.

Fixtures are accented throughout (`révision-de-plan`, `tâche-a2a-1`, a French
verdict string). This repo's nominal population of agent names, plan titles and
paths is French; an ASCII-only fixture does not test the traffic we run.

## Boundaries

- **Not a delivery guarantee.** This makes an answer that was *already produced*
  reclaimable. It does not queue, re-send, or promise delivery.
- **`a2a_call` is unaffected in practice.** The builtin tool declares
  `timeout_secs() = 120` at the tool layer, so the 300 s client budget is an
  outer bound that never fires there.
- **The `llm response body` log line is not a contract.** It was the manual
  safety net on 2026-08-29; it is not designed for recovery and must not become
  the mechanism.
- **Sibling, one layer down:**
  `runtime-errors/llm-http-timeout-covers-streaming-generation-2026-07-26.md`
  (mika#1660) bounds the *LLM provider* call. Same family — a long generation
  outliving a transport budget — but a different hop, and that fix had no
  recovery half because at that layer there is nothing persisted to reclaim.
