# RT-005 orchestration — 80 runs behind a manip-check gate (brick 3/5)

Orchestrates the RT-005 physics pilot: the 2x2 design (injected confidence
{0.95, 0.55} x peer reliability {fiable, degradee}), 10 items, R=2, **80 runs**
in a seeded random order, one at a time.

Ticket: mika#1890. Siblings: mika#1887 (`peer_b`), mika#1888 (the two confidence
agents), mika#1889 (`turn_usage` logging), mika#1891 (the offline analyser).

**This script computes nothing.** No mean, no delta, no comparison across cells.
It never parses a token count — `turn_usage` lines are copied verbatim. Analysis
is mika#1891's job, and the separation is enforced by a test, not by a promise.

## Modes

```bash
# Expand, order and record the plan. Zero LLM calls. Works on a host with no
# mika binary and no provisioned agents.
bash run-batch.sh dry-run --out-dir /tmp/rt005-dry

# Live paired runs producing evidence for YOU on whether each knob moves.
bash run-batch.sh manip-check --out-dir ~/.mika/rt005/<batch-id>

# The 80 runs. Refuses to start unless you have opened the gate.
bash run-batch.sh batch --out-dir ~/.mika/rt005/<batch-id>
```

Prerequisites for the two live modes: `bash ../confidence-agents/provision.sh`
(mika#1888), and a `mika` binary — resolved from `MIKA_BIN`, then `PATH`, then
`~/.local/bin/mika`, because a non-interactive shell on this host has no `mika`
on `PATH`.

## The gate

The batch is fail-closed. It starts only when **all** of these hold:

| Factor | Checked against |
|---|---|
| `manip-check/manip-check.json` exists and is readable | the batch directory |
| its `coverage` is `complete` | four cells that each produced a **successful** run; a capped or all-failed check is `partial` |
| its `design_fingerprint` | the recomputed fingerprint — the evidence must come from the design about to be spent |
| `PREREGISTRATION.md` exists | its SHA-256 goes into the manifest before run 1 |
| `AUTHORIZATION`'s `batch_id` | this batch |
| `AUTHORIZATION`'s `design_fingerprint` | the recomputed fingerprint |
| `AUTHORIZATION`'s `manip_check_sha256` | the artifact's actual digest |
| `AUTHORIZATION`'s `verdict` and `dated` | non-empty after whitespace is stripped |

Any failure exits 3, **names the field that failed**, and makes no LLM call.
Fields are plaintext on purpose: a single opaque token would make every denial
mute about which factor drifted, and would buy no property this gate has.

You write `AUTHORIZATION` yourself. The script never writes it, in any mode —
there is a test asserting no such file exists after a manip-check, and another
asserting the script source contains no write to that path.

```
batch_id: rt005-20260901
design_fingerprint: <from the manip-check output — not from the batch's own manifest>
manip_check_sha256: <sha256sum manip-check/manip-check.json | cut -d' ' -f1>
verdict: <what the manip-check showed, and why it licenses 80 paid sessions>
dated: 2026-09-01
```

### What this gate does not do

It makes the batch impossible to start **by accident**, **by inertia**, **by a
rerun of a previous command**, and **under a design that drifted since the
check**. The design fingerprint hashes the ordering seed, peer_b's seed, the
replicate count, the cell set, the agent names, and a canonical hash of the
*entire* bridge output — so a changed prompt or a changed peer answer moves it
even when every item id is unchanged. The manip-check artifact carries that same
fingerprint and the gate compares it, so evidence gathered under one design
cannot authorize a spend under another.

It does **not** stop a party that sets out to satisfy it. Every input is inside
the batch directory; anyone who chooses to compose an authorization can. That
matters concretely here: in this workspace the party that would launch the batch
is often an autonomous session, for which "impossible by accident" holds and
"impossible by decision" does not.

So the gate is one of two layers. The structural one is here. The other is the
operating rule that launching the 80 runs is the operator's call, and that
composing an authorization on the operator's behalf is a bypass, not a
shortcut. Neither layer is claimed to be the other. There is deliberately no
`--force` and no environment-variable escape hatch; adding one would remove the
only property the structural layer has.

## What the manip-check answers, and what it cannot

Two questions, both by **live paired runs** on an item peer_b actually perturbs:

1. Does the 0.95 / 0.55 injection move behavior? (high vs low, at each arm)
2. Does the reliability knob produce something the agent distinguishes?
   (fiable vs degradee, at each confidence level)

Neither is answered structurally. `PeerB::with_fixture` refuses to build a
degraded instance that perturbs nothing, so "the arms' answers differ" is true
by construction and could never register a failure — a gate factor that cannot
fail carries no information, and an earlier draft of this script had exactly
that hole.

The artifact records paired raw outputs, the raw `turn_usage` lines per run
(the primary outcome's own scale), binary identical/not-identical observations,
and the realised perturbation rate per cell. **It draws no conclusion.** You do,
in the `verdict` field.

`--limit N` caps the live runs. A capped check is marked `coverage: partial` and
cannot open the gate whatever authorization is written against it — that is what
lets this script be verified without spending the budget.

## The dilution — read this before reading any result

`peer_b` perturbs `n * 2 / 6` items: **3 of 10**. A run's prompt carries only the
item and peer_b's answer, so on the other 7 items the prompt is **byte-identical
across the two reliability arms**. Of the 20 runs in each degraded cell, **6**
carry a wrong peer answer and **14** are input-identical to their fiable
counterparts.

The design is balanced on the *label*, not on the *manipulation*. This is a
property of the ratified protocol, not of this script, and it is reported in the
manifest and on the manip-check artifact so no reader infers 20 manipulated
observations from "20 per cell".

Two contrasts are pre-registered, both before any data existed (operator
decision, 2026-08-29):

- **Primary — the labelled arm** (20 vs 20). This protocol's intention-to-treat
  estimand. The estimand is not changed after the fact.
- **Secondary, pre-specified — the realised perturbation** (6 vs 6, with the
  remaining 14 pairs as within-design controls).

Both are reported. Reporting only one is a protocol violation whichever one it
is.

## Claim boundary

**Existence, not magnitude.** The knob is synthetic — a stub answering
deliberately easy items — so external validity is bounded by construction. Any
number from this batch is a property of the apparatus, not an estimate of a
quantity in the world. The disclaimer is written into the manifest, the
manip-check artifact and every run record.

## Batch directory

```
manifest.json                  design, seeds, order, prereg hash, contrasts,
                               per-cell dilution, agent-state fingerprints,
                               operator verdict, disclaimer
plan.json                      the 80 expanded runs, in order
peer_b.json                    the bridge's output, verbatim
manip-check/manip-check.json   paired raw outputs + raw turn_usage + coverage
AUTHORIZATION                  operator-written; never written by the script
runs/<run_id>.json             one record per run: status, attempt, perturbed
logs/<run_id>.turn_usage.jsonl that run's raw turn_usage lines
```

Run ids are `<confidence>.<reliability>.<item>.r<replicate>`.

## Notes for whoever runs this

**Resume is safe.** Only a record that is *successful* **and** carries this
design's fingerprint suppresses re-execution — a record left by a run at another
peer seed is re-executed rather than counted as an observation of this design.
Failed runs are re-attempted. The attempt is claimed in an `in_flight` record
written **before** the paid call, so a crash between the call and the result
cannot leave the counter unadvanced and silently merge two paid turns into one
record. Records are written atomically and a producer that fails writes nothing
rather than truncating its destination.

**Where the tokens actually live — read this before changing the capture.**
Since mika#1727 `mika ask` does not run the agent loop in process. It is an A2A
client that posts the prompt to mika-spirit, which owns the execution session,
and the per-agent CLI log carries no `turn_usage` for an `ask`.

Since mika#2070 the CLI's `--session-id` **does** cross the wire, in
`message/send` request metadata, and spirit runs the turn under it when it owns
that session row. A `turn_usage` event therefore carries the run's own session
id, and correlation no longer has to be inferred. The byte-offset slice below
predates that fix and is kept as the belt to its braces — it is what catches a
deployment where the fix is not yet live.

`mika/CLAUDE.md` Signal O used to say to read
`~/.mika/agents/<name>/logs/mika.log.$(date +%F)` for a `mika ask`. That was true
before #1727 and stale afterwards; following it captured nothing. **mika#2069
corrected it** — Signal O now names `$MIKA_SPIRIT_LOG_FILE` as the sink for every
`turn_usage`, entry door included, and states the reason. The measurement channel
is spirit's own log (`MIKA_SPIRIT_LOG_FILE`, else `/var/log/mika/server.log`).

So each run is still correlated by **(agent_id, byte-offset slice)**: the script notes
the log size before the call and reads only what was appended after it. This is
sound because the batch is strictly sequential and these two agents exist only
for this experiment. It is also checked rather than assumed — if the slice
carries more than one spirit session for that agent, another conversation
overlapped the run and the record is marked `contaminated` instead of being
attributed.

**A run that logs nothing is a failure**, recorded `failed` with a named reason
rather than filed as an empty file. And the live preamble probes the channel
before spending anything: if spirit's log holds no `turn_usage` line at all, the
run aborts rather than discovering it 80 sessions later.

**The circuit breaker is the budget's real protection.** Three consecutive
failed runs abort the batch. Any systemic cause — spirit down, a broken capture
channel, a provider outage — then costs three sessions instead of eighty.

**The subject may not be stationary.** Core memory is DB-backed per agent and
injected into the system prompt across sessions, and these agents carry
`search_memory` and a soul that tells them to persist facts — so anything
written in run 3 is in the system prompt for run 40. A fresh session id does not
prevent that. The manifest fingerprints each agent's persistent state before run
1 and after run 80; a change means the subject drifted and the batch should be
read accordingly.

**A known limit of the fixture, for whoever reads the outputs.** peer_b perturbs
by substituting *another fixture item's* answer, which keeps the wrong answer
well-formed but can make it type-incongruent with the question — the degraded
answer to "Sum of 47 and 68" is `akim`. An agent may reject that on shape rather
than on reasoning about its peer's reliability. This is brick 1's design
(mika#1887) and out of scope to change here; it bears on how much of any
observed effect is "distrust of an unreliable peer" versus "this string is not a
number".

## Scaling

If power is short, follow `PREREGISTRATION.md` — R=2→R=3 first, 10→15 items only
if replication does not resolve it.

A scale-up is a **new batch**: a new `--out-dir`, a new `--batch-id`, its own
manip-check and authorization, and `--extends-batch <prior id>` so the manifest
records the lineage. The prior batch's directory is never rewritten, because the
authorization binds to the replicate count and changing R in place would leave a
manifest claiming an order its runs were not executed in.

Note the honest limitation: `--replicates 3` plans all 120 runs in a fresh
seeded order, not only the 40 added ones. Deduplicating against the prior batch
is a manual step — copy the prior batch's successful `runs/*.json` into the new
directory before authorizing, and the resume logic will skip exactly those whose
design fingerprint still matches.

## Tests

```bash
bash tests/test_run_batch.sh          # no test makes an LLM call
cargo test --example rt005_batch_plan # the bridge
```

`mika` is replaced by a stub, so the gate, the resume path and the capture path
are all exercised without spending the budget.
