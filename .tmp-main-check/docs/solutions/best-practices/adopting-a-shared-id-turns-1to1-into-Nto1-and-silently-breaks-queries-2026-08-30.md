---
module: agent-core
tags: [cardinality, a2a, sessions, sql-scoping, mutation-testing, seams, turn-usage]
problem_type: silent-logic-error
category: best-practices
---

# Adopting a shared id turns 1:1 into N:1, and the queries that assumed 1:1 break silently

## Problem (mika#2070)

`turn_usage` carried mika-spirit's own `a2a-<task_id>` session instead of the caller's, so RT-005 could only attribute a run's turns by time slice. The fix looked contained: carry the caller's session id over `message/send`, adopt it when the agent already owns that session row.

It was contained. The breakage was somewhere else entirely.

Before the change, an A2A task always got a **freshly minted** session — one session, one task. `a2a_get_messages` read a task's history with `WHERE session_id = ?1` and no task filter, which was *exact* under that invariant. Adoption made the relation many-to-one, and that query became wrong without being touched:

```
mika ask --session-id S   →  task-1  →  messages(S) = [ask1, reply1]
mika ask --session-id S   →  task-2  →  messages(S) = [ask1, reply1, ask2, reply2]
                                          ↑ render_task_parts concatenates BOTH agent replies
```

Not hypothetical: `skills/bundled/_shared/dispatch-lib.sh` reuses one session on its grooming retry **on purpose**, so the architect can see its own prior turn. The retry would have returned both replies concatenated, and the verdict parser would have read the wrong disposition.

## What to check

When a change makes an identifier **shared** where it used to be minted per-unit — a session, a trace, a correlation key, a directory, a lock — the diff is not the blast radius. Grep every query and every read path that filters on that identifier and ask, for each one, *what made this filter exact?* If the answer is "there was only ever one," that call site is now wrong and nothing will fail loudly.

Here the fix was to scope the read by the unit, not the container: the agent loop already stamps the A2A task id as each message's `trace_id`, so the filter matches that (and `metadata.a2a_task_id` for the other writer, so tasks predating adoption stay readable).

The tell is a comment or a name that encodes the old cardinality. `a2a_create_task` returning "the session_id for use with the agent loop" reads as 1:1 and stops being true the moment a caller can name the session.

## Second finding: the seams between separately-tested layers hold nothing

Extraction was unit-tested. Adoption was unit-tested. CLI emission was unit-tested. Then a reviewer mutated **both server call sites to pass `None`** — the whole feature disabled — and `cargo test -p mika-agent --lib` returned `4066 passed; 0 failed`.

Three green units do not make a working chain. Each unit test pins its own function; nothing pins the argument actually being forwarded. The cheap close is one test through the real entry point: here, POSTing a JSON-RPC `message/send` body through the router with `returnImmediately: true` — which binds the session without starting the agent loop or paying for an LLM call — then asserting the task bound to the caller's session.

**A test you have not seen fail is a claim, not evidence.** Delete the fix, run the test, watch it go red — that is what makes it coverage. See [[a-stub-built-from-the-doc-cannot-falsify-the-doc]] for the same discipline applied to fixtures.
