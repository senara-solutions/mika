---
module: research
tags: [measurement-channel, stale-documentation, test-stubs, topology-change, a2a, turn-usage, rt-005, observability]
problem_type: silent-data-loss
category: best-practices
---

# A stale doc plus a stub that agrees with it hides a dead measurement channel

> **State note (mika#2070, 2026-08-30).** The topology described below is the one
> that existed when this was written, and the lesson is unchanged. One fact has
> since moved: the CLI's `--session-id` now *does* cross the wire, in
> `message/send` request metadata, and spirit runs the turn under the caller's
> session when it owns that session row — so a `turn_usage` event names its own
> run. The rest holds: the measurement channel is still spirit's log, not the
> per-agent CLI log, and Signal O in `mika/CLAUDE.md` is still stale.

## Problem (mika#1890)

RT-005's brick 3/5 orchestrates 80 paid LLM sessions and captures each run's
`turn_usage` lines — the study's primary outcome. The capture was built from the
repo's own operator documentation, `mika/CLAUDE.md` Signal O:

> **CLI vs server sinks:** for a `mika ask` invocation, replace
> `$MIKA_SPIRIT_LOG_FILE` with `~/.mika/agents/<name>/logs/mika.log.$(date +%F)`

That sentence was true when it was written. It stopped being true at mika#1727,
which turned `mika ask` from an in-process agent loop into a thin A2A client:
the prompt now goes to mika-spirit, **spirit** owns the execution session, and
`ask.rs` says so in its own comment — *"the local bookkeeping session created
above no longer records agent turns (spirit owns the execution session)"*. Only
the message text crosses the wire, so the CLI's `--session-id` never reaches
`emit_turn_usage`, and the per-agent CLI log carries no `turn_usage` for an
`ask` at all.

Measured on the host: 21 056 `turn_usage` lines in `/var/log/mika/server.log`,
all under spirit-minted session ids; the `turn_usage` lines in
`~/.mika/agents/<name>/logs/mika.log.*` are `heartbeat-` and `reflection-`,
none from an `ask`.

Had the batch run, all 80 sessions would have been billed, every capture would
have been empty, every run would have been recorded `failed`, and the loss would
have surfaced at the offline analysis stage — after a budget that can be spent
once per design was gone.

**The test suite could not see it.** 120 assertions passed, including
"every run captured its `turn_usage` lines" and "each capture holds only its own
session's lines". The stub wrote a `turn_usage` line keyed by the `--session-id`
it was handed, into the per-agent log directory — because it was written from
the same stale sentence as the code. A stub built from the documentation cannot
falsify the documentation. The green suite was measuring the author's belief
about the topology, twice.

## Solution

**When a doc tells you where a signal lands, go look at the signal before you
build on it.** One `grep -c` in each candidate sink, and one look at the field
you intend to correlate on, would have cost a minute:

```bash
grep -c turn_usage /var/log/mika/server.log            # 21056
cat ~/.mika/agents/mika-dev/logs/mika.log.* | grep -c turn_usage   # heartbeat only
grep turn_usage /var/log/mika/server.log | tail -1 | jq '.fields.session_id'
```

**Then write the stub from the observation, not from the doc.** The corrected
stub appends to a stand-in *spirit* log under a session id it mints itself,
ignoring the one it was passed — modelling the topology that exists rather than
the one that is described. Every capture assertion that passed before now passes
against a channel that is real.

Three rules generalise:

1. **A doc sentence about a runtime path is a claim with a timestamp, not a
   fact.** Prefer the code that emits the signal and the file that holds it. A
   comment left by the change that broke the doc (`ask.rs` had one) is worth
   more than the doc it invalidated.
2. **A stub is only evidence about the world if it was built from the world.**
   When the stub and the code under test share a premise, the suite tests the
   premise's internal consistency. Ask, of every green suite protecting a
   measurement: *what observation would have to be true for this stub to be
   right, and did anyone check it?*
3. **Correlate on a key the producer actually emits.** The design keyed runs to
   token lines by a session id the caller chose; the producer emits one it mints
   itself. Where an exact key is unavailable, an inferred correlation
   (here: agent id plus a byte-offset slice, valid because the batch is
   sequential) must be *checked* — the corrected capture marks a run
   `contaminated` when its slice carries more than one producer session, rather
   than attributing it.

## The doc is still stale

`mika/CLAUDE.md` Signal O still names the per-agent path. This brick did not
change it — out of scope for mika#1890, and surfaced to the operator instead.
Anyone measuring tokens from a `mika ask` should read spirit's log
(`MIKA_SPIRIT_LOG_FILE`, else `/var/log/mika/server.log`) until that line is
corrected.

The durable fix is upstream of both: thread the caller's session id through
`message/send` into `AgentParams.session_id`, so correlation becomes exact
instead of inferred.

## Related

- mika#1727 — the topology change that invalidated the doc.
- mika#1889 — `turn_usage` instrumentation (the signal itself is fine; only its
  documented sink was wrong).
- `docs/solutions/best-practices/1887-mutate-the-knob-a-passing-test-suite-does-not-prove-a-manipulation-works.md`
  — the sibling failure at the other end of the same experiment: there, a green
  suite did not prove the manipulation worked; here, a green suite did not prove
  the measurement worked. Both are the same shape — the suite agreed with the
  author instead of with the system.
