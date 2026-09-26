---
title: RT-005 Orchestration Script — 80 Runs, Randomized, Behind a Manip-Check Gate (Brick 3/5) - Plan
type: feat
date: 2026-08-29
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# RT-005 Orchestration Script — 80 Runs, Randomized, Behind a Manip-Check Gate (Brick 3/5) - Plan

## Goal Capsule

**Objective.** The RT-005 operator can spend the 80-run budget once, deliberately,
and on a manipulation that was shown to move before the budget was committed —
and a reader of the resulting artifacts can tell, from the artifacts alone, what
the batch was allowed to claim, what decision rule was fixed before any data
existed, and how much of the design was actually manipulated rather than merely
labelled.

**Means.** A bash orchestrator under `research/rt005-physics-pilot/orchestration/`
whose full-batch mode is fail-closed behind an operator-written authorization
bound field-by-field to a specific manip-check and a specific design (KTD3), fed
by a minimal Rust bridge that supplies only what bash cannot compute — peer_b's
answers (KTD1).

**Authority hierarchy.** Issue mika#1890 `## Scope`, its four non-negotiable
guardrails, and its `## Acceptance criteria` are authoritative. The RT-005 Round
Table ratification (2026-07-28) fixes the 2x2 design and the 80-run cardinality.
Sibling bricks mika#1887 (peer_b, merged), mika#1888 (confidence agents, closed)
and mika#1889 (`turn_usage` logging, merged) are fixed inputs — this brick reads
their surfaces and changes none of them. Brick 5/5 (mika#1891) owns all analysis.

**Stop conditions.** Stop at the orchestrator, its bridge, its pre-registration
document, its README, and its tests. Do not run the 80-run batch — the ticket is
`operator-gated` for execution, not for construction. Do not modify
`crates/mika-agent/src/research/peer_b.rs`, the mika#1888 agent configs, the
agent loop, or `crates/mika-agent/Cargo.toml`. Do not compute a statistic.

**Execution profile.** One bash script, one cargo example, one pre-registration
document, one README, one test suite. No migration, no config key, no product
surface. The whole gate is
`bash research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh` plus a
zero-LLM dry run of the full 80-run plan on an unprovisioned host.

**Tail ownership.** PR opened against `senara-solutions/mika` with `Closes #1890`
and reviewer `mika-platform-qa`.

## Product Contract

### Summary

Add `research/rt005-physics-pilot/orchestration/`: an orchestrator that expands
the RT-005 2x2 design into 80 fully-specified runs in a seeded random order,
executes them one at a time against the two mika#1888 confidence agents, and
files each run's raw record and raw `turn_usage` lines under a per-batch
directory. Its full-batch mode refuses to start unless the operator has written
an authorization that names this batch, this design, this manip-check, and what
the operator concluded from it. It computes nothing.

### Problem Frame

RT-005 estimates whether injected confidence interacts with real peer
reliability in how an agent plans. The batch costs 80 paid LLM sessions and can
be spent exactly once per design. Three failure modes make that spend worthless,
and none announces itself.

The first is an inert manipulation. If the 0.95-vs-0.55 prior does not actually
move behavior, or the reliability knob produces nothing the agent can
distinguish, every run still completes, still logs, still yields a clean
dataset — and the batch measures noise while reporting a null. mika#1887 closed
this at the module level for the reliability arm; what remains unproven is
whether the arms differ *in the agent's behavior*, which only a live check can
show. A manip-check that runs after the batch, or that the batch can start
without, protects nothing.

The second is a design that is balanced on its labels rather than on its
manipulation. peer_b perturbs `n * 2 / 6` items, so on the ten-item fixture the
degraded arm answers three items wrongly and seven correctly. Because a run's
prompt carries only the item and peer_b's answer, those seven items produce a
byte-identical prompt in both reliability arms: of the twenty runs in a degraded
cell, six carry a wrong peer answer and fourteen are input-identical to their
fiable counterparts. That is a real property of the ratified design, not a
defect to fix here — but an artifact that reports "20 per cell" without it
invites a reader to infer twenty manipulated observations.

The third is retrofitted inference. A scaling rule written after seeing the data
is a description of the data, not a decision rule; and a synthetic knob bounds
external validity, so the honest claim is that the interaction *exists*, never
how large it is. Both are usually left to the write-up, where they decay. This
brick fixes all three in the artifacts the batch itself produces.

### Requirements

Design and order:

R1. Expand the design into exactly 80 runs — confidence {high, low} x
reliability {fiable, degradee} x 10 fixture items x R=2 replicates — with every
cell populated exactly 20 times.

R2. Order the 80 runs by a seeded permutation. The same seed and design yield a
byte-identical order; a different seed yields a different one. The seed and the
resulting order are recorded in the batch manifest before the first run.

R3. peer_b's construction seed is a separate, pinned value, distinct from the
ordering seed, recorded in the manifest and covered by the design fingerprint.
Changing the ordering seed reshuffles the order and changes nothing else.

R4. Each run record carries whether peer_b's answer for that run was perturbed,
as a fact read from peer_b's realised `perturbed_ids` — not as a computed
statistic.

The gate:

R5. Full-batch mode is fail-closed. In the absence of a valid operator
authorization the batch does not start. Missing, empty, unreadable, malformed,
stale, or mismatched authorization all deny. There is no environment variable,
flag, or argument that skips the gate.

R6. The authorization binds in named plaintext fields — batch id, design
fingerprint, manip-check digest — each compared against the recomputed value.
A denial names the field that failed.

R7. The authorization carries a non-empty operator `verdict:` stating what the
manip-check showed and why it licenses the spend, and a `dated:` line. Both are
copied verbatim into the manifest. The script judges presence and non-emptiness
only; it reads nothing into the text.

R8. The script never writes an authorization file, in any mode.

R9. A manip-check that did not cover all four cells is marked partial, and a
partial manip-check cannot open the gate.

The manip-check:

R10. A manip-check produces evidence for the operator on two questions — does
the 0.95/0.55 injection move behavior, and does the reliability knob produce
something the agent distinguishes — and decides neither. Both questions are
answered by live paired runs; neither is answered by a structural property of
peer_b.

R11. Each manip-check run captures its raw `turn_usage` lines exactly as a batch
run does, so the operator weighs the evidence on the dependent variable's own
scale rather than on prose differences.

Execution:

R12. Each run invokes the correct agent (`mika-dev-confidence-high` or
`mika-dev-confidence-low`) against peer_b's answer for that item under that
reliability arm, and captures that run's `turn_usage` lines.

R13. The confidence factor never enters the user message. The prompt text for a
given (item, reliability) pair is byte-identical across the two confidence
cells; the prior reaches the agent only through the mika#1888 `soul.md`.

R14. A failed run is recorded as failed, atomically, with its captured stderr. A
killed process leaves no partial record. A run whose capture yields zero
`turn_usage` lines is an error, not an empty file.

R15. Reruns are idempotent: a run whose successful record already exists is
skipped rather than re-executed, so an interrupted batch resumes without
double-spending.

R16. A retried run opens a fresh session — the session id carries an attempt
component — so a retry never resumes the partially-consumed session of the
attempt it replaces.

R17. The persistent state of each agent is fingerprinted before the first run
and after the last, and both fingerprints are recorded. A change means the
subject was not stationary across the batch.

Output discipline:

R18. The script produces no aggregate: no mean, no delta, no ratio, no
comparison across cells, no summary statistic. It never parses a token count —
`turn_usage` lines are copied verbatim. Only per-run raw records and the
manifest.

R19. Every artifact a reader could mistake for a result — the manifest, the
manip-check artifact, and each run record — carries the existence-not-magnitude
disclaimer in its header.

R20. The pre-registration document exists before the first run, states the
scaling rule and the claim boundary, and its hash is written into the manifest
before the first run. A missing pre-registration document denies the batch.

R21. A dry run expands, orders, and records the full 80-run plan while making
zero LLM calls, and succeeds on a host with no `mika` binary and no provisioned
confidence agents.

R22. A scale-up under the pre-registered rule is a new batch that extends the
prior one by id, with its own manifest, order, manip-check and authorization.
The prior batch's manifest and order are never rewritten.

R23. Both analysed contrasts are engraved in the manifest, named as
pre-registered before data: the labelled arm as primary (intention-to-treat) and
the realised perturbation as the pre-specified secondary, with the rule that
reporting only one is a protocol violation.

R24. The realised perturbation rate is reported per cell, on the manifest and on
the manip-check artifact the operator reads before authorizing. It is counted
from peer_b's realised `perturbed_ids` — a design fact, not a measurement, so
R18 is untouched.

R25. The manip-check artifact records the design it was run under, and the gate
refuses an authorization whose evidence describes a different design.

R26. `coverage: complete` counts cells that produced a **successful** run. A
check whose runs all failed is `partial`, cannot open the gate, and reports
nothing as comparable — an empty failure must not read as evidence of no effect.

R27. The batch aborts after three consecutive failed runs. A systemic cause must
cost three sessions, not eighty.

R28. A run whose captured slice carries more than one spirit session for its
agent is recorded `contaminated`, not attributed.

R29. A record suppresses re-execution only when it is successful **and** carries
the current design fingerprint.

R30. The attempt is claimed in an `in_flight` record written before the paid
call, so a crash between the call and the result cannot leave the counter
unadvanced and merge two paid turns into one record.

R31. A denied batch leaves the output directory as it found it — no manifest,
and no overwrite of a committed `plan.json` or `peer_b.json`.

R32. Manip-check runs are recorded under `manip-check/runs/`, not in the batch's
`runs/`, so pre-authorization runs cannot silently satisfy part of the 80.

### Key Decisions

KD1. The gate is a deliberation barrier, not an access control, and the README
names who it restrains. See KTD3.

KD2. Analysis belongs to brick 5/5. This script's refusal to compute is a
product requirement (R18), not an implementation shortcut.

KD3. The dilution in R4 is a property of the ratified design, surfaced rather
than corrected. Changing peer_b's `2/6` rate would be a change to a merged
sibling brick and to the ratified protocol; naming the dilution in the
pre-registration, in each run record, and per cell on the manip-check artifact is
this brick's whole responsibility for it.

KD4. The estimand was settled by the operator on 2026-08-29, before any data
existed: the **labelled arm** is primary (this protocol's intention-to-treat
contrast, not changed after the fact) and the **realised perturbation** is a
pre-specified secondary contrast. Both are reported. Making the dilution visible
is exactly what the manip-check is for, which is why R24 puts the per-cell rate
on the artifact the operator reads before authorizing rather than only in the
pre-registration.

### Acceptance Examples

AE1. Operator runs full-batch mode in a fresh batch directory with no
authorization file. The script exits non-zero, names the missing gate, and makes
zero LLM calls.

AE2. Operator runs a manip-check limited to two runs. The artifact is written
with `coverage: partial`. Full-batch mode, presented with an authorization bound
to that artifact, still denies.

AE3. Operator completes a four-cell manip-check, writes the authorization, and
then changes the batch seed. Full-batch mode denies and names
`design_fingerprint` as the field that failed.

AE4. A dry run over the default design, on a host with no `mika` on `PATH` and
no confidence-agent directories, writes a manifest with 80 runs, 20 per cell, in
seeded order, and creates no run record.

AE5. The same ordering seed produces the same order twice; a different one does
not; peer_b's answers and `perturbed_ids` are identical across both.

AE6. For one item under one reliability arm, the prompt sent to the high agent
and the prompt sent to the low agent are byte-identical.

AE7. A manip-check completes with a stubbed `mika` and no authorization file
exists in the batch directory afterwards.

### Scope Boundaries

In scope: the orchestrator script, the Rust bridge that exposes peer_b's answers
to bash, the pre-registration document, the README, and the test suite.

Out of scope and explicitly not built: executing the 80-run batch (operator-held,
post-gate); any statistic, plot, or comparison (brick 5/5, mika#1891); any change
to `peer_b.rs`, to the mika#1888 configs, to the agent loop, or to
`crates/mika-agent/Cargo.toml`; provisioning the confidence agents (the mika#1888
`provision.sh` already does that and the orchestrator only checks its result);
any scheduler, queue, or parallelism.

**Settled by the operator, 2026-08-29.** Whether this brick owns *conducting*
the manip-check or only *refusing without one* was routed and answered: the mode
stays in the script, because a check conducted by a different route than the
batch does not verify the thing that will run — same apparatus, same prompts,
same capture path. The `--limit` cap plus `coverage: partial` stays as the guard
that stops a capped check from opening the gate on its own.

### Dependencies

Upstream: mika#1887 (merged, `origin/main`), mika#1888 (closed, config assets in
tree), mika#1889 (merged). Downstream: mika#1891 consumes this batch's raw
records. Runtime: `jq`, `sha256sum`, `cargo`, and — for live modes only — a
`mika` binary, which lives at `~/.local/bin/mika` and is absent from
non-interactive shell `PATH`, so it is resolved explicitly (KTD5).

## Planning Contract

### Key Technical Decisions

KTD1. **A minimal cargo example bridges bash to peer_b, and carries nothing bash
can compute.** `crates/mika-agent/examples/rt005_batch_plan.rs` emits one JSON
document holding only what requires the Rust module: the fixture items with
their prompts, peer_b's committed answer per (reliability arm, item) at `k = 1`,
the realised `perturbed_ids` per arm, and the pinned peer_b seed. Run keys, the
permutation, the design fingerprint, and prompt assembly stay in the script,
where the orchestration belongs.

Cargo auto-discovers `examples/*.rs`, so this adds no `Cargo.toml` entry, no
runtime surface, and no API surface, and it deletes with the rest of the RT-005
scaffold. It does become a standing `--all-targets` build and lint target in
`mika-agent` for as long as the scaffold lives; that cost is real and is the
price of not reimplementing peer_b's answer selection in a second language,
where the two copies could silently diverge. mika#1887's plan put "a CLI
subcommand" out of its own scope and named mika#1890 as the owner of the
orchestration surface, so the bridge belongs to this brick rather than being a
retrofit of brick 1.

Its unit tests live as a `#[cfg(test)] mod tests` inside the example and run
under `cargo test --example rt005_batch_plan` — a named `--example` target is
tested regardless of the `test = false` default for examples, so no
`Cargo.toml` entry is needed for the tests either.

KTD2. **The run order comes from sorting on a hash of the seed and the run key,
computed in bash.** For each run, `key = sha256("<ordering_seed>:<run_key>")`;
runs sort by `(key, run_key)`. `sha256sum` is already a hard dependency of the
gate, independent keys give a uniform permutation, and the tie-break on
`run_key` makes the order total. There is no generator state to reproduce and
nothing hand-rolled.

peer_b's `SplitMix64` is private, and both making it public and copying it into
the bridge are worse than not needing it: the first widens brick 1's surface for
a consumer's convenience, the second creates two copies of a generator whose
whole reason for being hand-written is that a silent change to it invalidates a
recorded batch.

KTD3. **The gate binds in named plaintext fields, and its limits — including who
it restrains — are stated rather than implied.** `AUTHORIZATION` is written by
the operator and carries `batch_id`, `design_fingerprint`, `manip_check_sha256`,
`verdict`, and `dated`. Full-batch mode proceeds only when the manip-check
artifact exists, is `coverage: complete`, **and records the same design
fingerprint the batch is about to run under**; the pre-registration document
exists; each authorization field equals its recomputed value; and `verdict` and
`dated` are non-empty once whitespace is stripped. Any failure denies, names the
offending field, and exits non-zero before the first LLM call and before
anything in the output directory is touched.

The manip-check's own fingerprint is a factor distinct from the authorization's,
and the difference is load-bearing: without it, a check run at another peer seed
produced an artifact whose digest opened a batch under a design that was never
checked — demonstrated during review at 76 live calls.

Plaintext fields rather than a derived digest, deliberately: the properties the
gate actually has — no accidental start, no start by inertia, no start from a
rerun of a previous command, no start under a design that drifted since the
check — are delivered by field comparison, and a single opaque token would make
every denial mute about which factor drifted. Hashing would buy an adversarial
property the gate does not have anyway.

What it does not do, stated in the README rather than left to inference: it does
not stop a party that sets out to satisfy it. The inputs are all inside the
batch directory, so anyone — operator or autonomous session — who chooses to
compose an authorization can. In this workspace the party that would launch the
batch is frequently an autonomous pilot session, for which "impossible by
accident" holds and "impossible by decision" does not. The gate is therefore
one of two layers: the structural one here, and the operating instruction that
launching is Vincent's call. Neither is claimed to be the other. Adding a
`--force` flag or an env-var bypass would remove the only property the
structural layer has.

R8 backs this with the cheapest bypass in mind: not a flag, but four lines
appended to the manip-check that write the authorization "since the operator
already ran the check". A test asserts no authorization file exists after a
manip-check, and a second asserts the script source contains no write to that
path.

KTD4. **`--limit` exists for the manip-check and does not exist for the batch.**
A capped manip-check is how this work is verified without spending the budget,
so the cap must be available; it must also be impossible to launder a two-run
check into an authorization. The artifact records realised coverage, and R9's
`coverage: partial` marking is what the gate reads. The batch mode parses no such
flag at all — a flag that is absent cannot be passed by mistake.

KTD5. **The preamble splits by what each mode actually needs.** The shared part
requires `jq`, `sha256sum` and `cargo` and invokes the bridge. The live part —
resolving the `mika` binary and checking the mika#1888 agent directories — runs
only in `manip-check` and `batch`. Putting the live preconditions in the shared
part would abort the dry run on exactly the host the plan describes, where the
confidence agents are not provisioned, defeating R21, which exists so
orchestration is verifiable without the live prerequisites.

`MIKA_BIN` overrides binary resolution; otherwise the script tries
`command -v mika`, then `$HOME/.local/bin/mika`, and fails with a named error.
The live preamble also asserts each agent's `config.toml` sets `log_level` to
`info` or finer: `resolve_log_level` falls back to `warn` when neither
`MIKA_LOG_LEVEL` nor a config value is set, and under `warn` the INFO-level
`turn_usage` event is filtered out and every capture file would be silently
empty. The mika#1888 `shared/config.toml` sets `info` today; the assertion makes
the dependency explicit rather than lucky.

KTD6. **Runs are correlated to token lines by `(agent_id, byte-offset slice)`
on mika-spirit's log — not by `session_id`, and not on the per-agent CLI log.**

Since mika#1727, `mika ask` no longer runs the agent loop in process: it is an
A2A client that posts the prompt to mika-spirit, which owns the execution
session. `crates/mika-cli/src/commands/ask.rs` says so directly — *"the local
bookkeeping session created above no longer records agent turns (spirit owns the
execution session)"* — and only the message text crosses the wire, so the CLI's
`--session-id` never reaches `emit_turn_usage`.

Measured before the correction: 21 056 `turn_usage` lines in
`/var/log/mika/server.log` under spirit-minted session ids, and none from an
`ask` in `~/.mika/agents/<name>/logs/mika.log.*`. **`mika/CLAUDE.md` Signal O
still documents the pre-#1727 per-agent path** — that stale line is where the
original design came from, and following it would have produced 80 paid sessions
with an empty capture on every one, discovered only at brick 5/5.

The script therefore notes spirit's log size before the call and reads only the
bytes appended after it, filtered by `agent_id`. Sound because the batch is
strictly sequential and these two agents exist only for this experiment — and
checked, not assumed: a slice carrying more than one spirit session for that
agent means another conversation overlapped the run, and the record is marked
`contaminated` rather than attributed (R28). The observed spirit session id goes
into the record so the attribution stays auditable.

Correcting this upstream — threading the caller's session id through
`message/send` into `AgentParams.session_id` — would make correlation exact
rather than inferred. That belongs to mika, not to RT-005, and is surfaced to
the operator rather than filed unilaterally.

Each run still carries its own `--session-id
rt005-<batch_id>-<run_id>-a<attempt>`, now for record identity and retry
bookkeeping rather than for correlation (R16, R30).

KTD7. **A run record is written atomically, claims its attempt before the
spend, and states its own status.** The script writes to a temporary file and
renames, refusing to move an empty or malformed file over the destination — a
bare `cat > tmp; mv` let a failing producer truncate the destination, and an
emptied record restarted the attempt counter onto a session id already used.

The attempt is claimed in an `in_flight` record written *before* `mika ask` is
invoked (R30). Writing it after the call left the counter unadvanced across a
crash in exactly the window the attempt counter exists for, so the retry reused
the session id it was meant to replace. A `mika ask` failure writes a record with `status: failed` and the captured
stderr; failed records are re-attempted on resume — only a successful record
suppresses re-execution. Without a stated policy, a failed run that wrote any
record would be skipped forever and the batch would reach "done" with fewer than
80 valid observations while looking complete.

KTD9. **The batch has a circuit breaker.** Three consecutive failed runs abort
it (R27). Without one, any systemic cause — spirit down, a dead capture channel,
a provider outage — costs the full 80 paid sessions and is reported in a single
summary line at the end. Three is deliberately low: for a budget that can be
spent once, stopping early and being wrong costs a rerun, while continuing and
being wrong costs the experiment.

KTD8. **The design fingerprint hashes the whole bridge output, not a list of
item ids.** `sha256` over `ordering_seed | peer_b_seed | replicates |
sha256(canonical bridge JSON) | sorted(cells) | sorted(agent_names)`, with
`LC_ALL=C` pinning collation. Hashing ids and a fixture length was a weak proxy
for the JSON the plan is actually built from: a changed prompt or a changed peer
answer left the fingerprint byte-identical while all 80 paid sessions asked
something different. R5's binding is only as
strong as this composition: a fingerprint over the seed and replicate count
alone would pass a narrow test suite while letting a batch run under a changed
item set or changed agent names with an authorization issued for a different
design — the silent carry-forward the binding exists to prevent.

### High-Level Technical Design

```
run-batch.sh <mode>
  |
  +- shared preamble: require jq/sha256sum/cargo; cargo run --example
  |     rt005_batch_plan -> peer_b.json { items[], answers[arm][item],
  |     perturbed_ids[arm], peer_b_seed }
  +- compose run keys, sort by sha256("<seed>:<run_key>") (KTD2),
  |     compute design_fingerprint (KTD8), assemble prompts (R13)
  |
  +- mode=dry-run ----> write manifest.json + plan.json, exit (zero LLM,
  |                     no mika binary or agent dirs required — R21)
  |
  +- live preamble (manip-check and batch only): resolve mika (KTD5),
  |     check agent dirs + log_level, fingerprint agent state (R17)
  |
  +- mode=manip-check -> paired live runs across the four cells, capturing raw
  |                      output AND raw turn_usage per run (R10, R11)
  |                      -> manip-check/manip-check.json (coverage:
  |                         complete|partial), writes NO authorization (R8)
  |
  +- mode=batch ------> GATE (KTD3): manip-check complete? prereg present?
                        each AUTHORIZATION field matches? verdict/dated
                        non-empty?  any NO -> deny naming the field, exit
                        non-zero, zero LLM calls
                        all YES -> write manifest, then for each of the 80 runs
                                   in plan order: skip if successful record
                                   exists (R15), else mika ask, file
                                   runs/<id>.json + logs/<id>.turn_usage.jsonl
                        -> closing agent-state fingerprint (R17)
```

Batch directory layout, all paths under `--out-dir` (default
`~/.mika/rt005/<batch_id>/`, outside the repo — 80 sessions of raw output is
data, not source):

```
manifest.json                  design, seeds, order, prereg hash, agent-state
                               fingerprints, operator verdict, disclaimer
plan.json                      expanded runs + the bridge's peer_b output
manip-check/manip-check.json   paired raw outputs + raw turn_usage, coverage
AUTHORIZATION                  operator-written; never written by the script
runs/<run_id>.json             one record per run, with status and perturbed
logs/<run_id>.turn_usage.jsonl raw turn_usage lines for that run
```

### Assumptions

A1. The 10-item fixture and R=2 are the ratified design; the script takes them
from peer_b and from a constant rather than inventing a second source of truth.

A2. Session isolation is asserted where it holds and checked where it does not.
A fresh session per run bounds in-conversation carry-over, but core memory is
DB-backed per agent and injected into the system prompt across sessions, and the
confidence agents inherit mika-dev's `search_memory` surface and a `soul.md`
that instructs them to persist facts. Anything written in run 3 is therefore in
the system prompt for run 40. R17's before-and-after fingerprints make that
drift visible in the artifacts instead of leaving it to an assumption the
session id cannot evidence.

A3. The manip-check pairs across one factor at a time, holding the item fixed:
the confidence pair varies `high` vs `low` at fixed reliability, and the
reliability pair varies `fiable` vs `degradee` at fixed confidence on an item
that peer_b actually perturbs. The reliability pairing is only informative on a
perturbed item — on the other seven the two arms are input-identical (R4), so
pairing on one of those would compare a run with itself.

A4. The operator provisions the mika#1888 agents with the existing
`provision.sh` before any live mode. They are not provisioned on this host
today, which is why the live preamble names the exact provisioning command and
why the dry run must not depend on it (KTD5).

### Risks & Dependencies

Risk 1 — the gate becomes ceremony. If a future edit adds a bypass for
convenience, the guardrail is gone and nothing visibly breaks. Mitigation: tests
assert batch mode denies under every enumerated gate factor, that the script
writes no authorization file, and that the source contains no bypass flag.

Risk 2 — an aggregate creeps in. A helpful summary line at the end of a batch
would violate R18 and read as a result. Mitigation: a test asserts the script
never references a token field (`input_tokens`, `output_tokens`,
`cache_*_tokens`) outside comments — the script copies `turn_usage` lines
verbatim and has no reason to parse one. This replaces an arithmetic grep, which
would have matched the loop counter over the 80 runs and been unsatisfiable.

Risk 3 — the bridge drifts from peer_b. If peer_b's answers change, a recorded
batch is no longer reproducible. Mitigation: the bridge is a thin caller with no
answer logic of its own, peer_b's seed is pinned and recorded (R3), and the
manifest records the design fingerprint a later reader can recompute.

Risk 4 — the batch is launched during verification of this very ticket.
Mitigation: verification is dry-run first, the live check is capped at two runs
via KTD4's `--limit`, and a capped check cannot open the gate.

## Implementation Units

### U1. Bridge — peer_b's answers as one JSON document

**Goal.** Give bash exactly what it cannot compute: peer_b's fixture, answers,
and realised perturbation.

**Requirements.** R3, R4, R12.

**Files.**
- `crates/mika-agent/examples/rt005_batch_plan.rs` (new)

**Approach.** Parse `--peer-seed`, defaulting to a pinned `PEER_B_SEED`
constant. Build `PeerB` for each reliability arm at that seed.

**No item filter, deliberately.** `peer_b` draws its perturbed subset from the
fixture it is handed, so constructing it over a subset would change *which*
items are perturbed and therefore change the manipulation itself. The bridge
always emits the whole fixture; selecting a few items for a manip-check is the
script's job, downstream of the answers. The argument parser rejects `--items`
rather than ignoring it.
Emit JSON: `peer_b_seed`, `items[]` (`id`, `prompt`, `truth`),
`answers` keyed by arm then item id (each `peer_b_solve(id, 1)`'s committed
answer), and `perturbed_ids` per arm. No ordering, no fingerprint, no prompt
assembly — those live in the script (KTD1).

**Test scenarios** (`#[cfg(test)] mod tests` inside the example, run by
`cargo test --example rt005_batch_plan`):
- The fiable arm's answers equal every item's ground truth and `perturbed_ids`
  is empty.
- The degradee arm's `perturbed_ids` has exactly 3 entries for the 10-item
  fixture, and its answers differ from the fiable arm on exactly those ids.
- Answers and `perturbed_ids` are identical across two invocations at the same
  peer seed, and differ at a different peer seed — the peer seed is
  load-bearing and is the only thing that moves the manipulation.
- Every emitted answer is a well-formed fixture answer, and no perturbed item's
  answer equals its own truth.
- The emitted JSON parses and carries all four top-level keys.

**Verification.** `cargo test --example rt005_batch_plan`, `cargo clippy
--all-targets -- -D warnings`, `cargo fmt --check`.

### U2. Pre-registration document and README

**Goal.** Fix the scaling rule, the analysed contrast, and the claim boundary in
writing, before any data exists, in a form the script can require and hash.

**Requirements.** R19, R20, R22, KD1, KD3.

**Files.**
- `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md` (new)
- `research/rt005-physics-pilot/orchestration/README.md` (new)

**Approach.** `PREREGISTRATION.md` states: the design and the primary outcome
(planning tokens, per mika#1889); the dilution fact from KD3 with its arithmetic
(`n * 2 / 6` → 3 of 10 items → 6 manipulated of 20 runs per degraded cell) and
which contrast is primary — labelled arm, with realised perturbation as a
pre-specified secondary contrast, both flagged as pending operator confirmation
before launch since the launch is gated anyway; the decision rule for
insufficient power (R=2→R=3 first, 10→15 items only if replication does not
resolve it, with the condition that triggers each) and how a scale-up is
executed as an extension batch (R22); and the existence-not-magnitude claim
boundary with its reason. It carries the date it was written and states it was
written before the first run. `README.md` documents the three modes, the gate,
and — per KD1 and KTD3 — what the gate does not stop and who it does not stop.

**Test scenarios.** Covered by U3: the batch denies when `PREREGISTRATION.md` is
absent; the manifest carries its sha256; and the file contains the scaling rule,
the dilution statement, and the claim boundary.

**Verification.** The U3 suite; manual read for the claim-boundary wording.

### U3. Orchestrator with the fail-closed gate

**Goal.** Expand, order, gate, execute, and record — computing nothing.

**Requirements.** R1-R22.

**Files.**
- `research/rt005-physics-pilot/orchestration/run-batch.sh` (new)
- `research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh` (new)

**Approach.** Three modes: `dry-run`, `manip-check`, `batch`. The shared
preamble requires `jq`/`sha256sum`/`cargo`, locates the repo root relative to
`$BASH_SOURCE`, invokes the U1 bridge once, composes the run keys, sorts them
(KTD2), computes the design fingerprint (KTD8), assembles the prompts (R13), and
writes `manifest.json` and `plan.json` — with the R19 disclaimer header, the
pre-registration hash, the two seeds and the ordered run ids.

`dry-run` stops there and requires no `mika` and no agent directories (R21). The
live preamble (KTD5) then runs for the two live modes. `manip-check` executes
the A3 pairs across the four cells, honoring `--limit` (KTD4) and marking
coverage, capturing raw output and raw `turn_usage` per run (R10, R11), and
writes no authorization (R8). `batch` runs the KTD3 gate before writing its
manifest and before any LLM call, exits non-zero naming the offending field on
any failure, and on success iterates the plan order, skips runs with a
successful record (R15), invokes `mika ask` with the per-run attempt-bearing
session id (KTD6), and files the run record (atomically, with `status` and
`perturbed` — KTD7, R4) plus that session's `turn_usage` lines.

**Test scenarios** (harness style matching `skills/bundled/_shared/tests/`, with
`mika` stubbed by a function so no test makes an LLM call):

Gate denials — each asserts exit 3, the named field, and zero `mika`
invocations:
- No `AUTHORIZATION` file (AE1).
- Empty `AUTHORIZATION`; whitespace-only `verdict` (R7).
- Unreadable `AUTHORIZATION` (a directory, and a mode-000 file) — denied as
  unreadable, never silently read as empty.
- A CRLF authorization denies on the real mismatch, not on a comparison whose
  two sides print identically.
- Field mismatches on `batch_id`, on `design_fingerprint`, and on
  `manip_check_sha256`.
- Empty `verdict`; empty `dated` (R7).
- Manip-check artifact absent; `coverage: partial` (AE2); recorded under a
  different design fingerprint (R25).
- A manip-check whose runs all failed: `coverage: partial`, nothing reported
  comparable, and the gate denies (R26).
- A seed change after authorization (AE3).
- `PREREGISTRATION.md` absent — exercised with a copy of the script at the same
  repo depth, since the path resolves from the script's own location (R20).

Gate integrity:
- No `AUTHORIZATION` exists after a manip-check run (AE7, R8).
- The script source contains no write to a path ending in `AUTHORIZATION`
  (redirection, `tee`, `cp`, `printf`) and no bypass flag (Risk 1).
- The script never references a token field outside comments (Risk 2, R18).

Expansion and order:
- Dry run on a `PATH` with no `mika` and no agent directories writes an 80-run
  manifest, 20 per cell, and creates no run record and no `mika` call (AE4, R21).
- The same ordering seed yields the same order; a different one does not; peer_b
  answers and `perturbed_ids` are unchanged across both (AE5, R3).
- The design fingerprint changes when the ordering seed, the peer seed, the
  replicate count, the item set, the cell set, or an agent name changes, and is
  stable otherwise (KTD8, R5).
- Prompts for the two confidence cells of one (item, arm) are byte-identical
  (AE6, R13).
- Each run record carries `perturbed`, true for exactly the degraded-arm runs on
  peer_b's `perturbed_ids` (R4).

Execution, capture and resume — the stub models the post-#1727 topology,
writing to a stand-in spirit log under a spirit-minted session id:
- Batch proceeds with a stubbed `mika` when every gate factor holds, writes 80
  records, and invokes the correct agent 40/40 times.
- Every record carries the disclaimer text, this design's fingerprint,
  `mode: batch`, and the spirit session it was attributed to (R19, R29).
- Each capture holds exactly one spirit session (KTD6).
- A second invocation re-executes nothing, and the resume preserves
  `agent_state_before` rather than recomputing it mid-batch (R15, R17).
- A record carrying another design's fingerprint is re-executed, not reused
  (R29).
- A failing stub trips the circuit breaker at three consecutive failures, and
  the resume re-attempts those three as attempt 2 with fresh session ids
  (R16, R27, R30).
- A run whose capture is empty is recorded `failed` with a named reason (R14);
  a slice carrying two spirit sessions is recorded `contaminated` (R28).
- `write_atomic` refuses an empty producer and malformed JSON, leaving the
  destination intact (KTD7).
- The live preamble aborts, with zero `mika` calls, on an unresolvable binary, a
  missing confidence agent, and a spirit log with no `turn_usage` line (KTD5).
- The manifest carries the pre-registration hash, the disclaimer, both
  agent-state fingerprints, the operator verdict and date verbatim, and both
  pre-specified contrasts with prose that tracks `--replicates` rather than
  hardcoding the R=2 numbers (R7, R17, R19, R20, R23).
- Manip-check records live under `manip-check/runs/`, marked `mode: manip-check`,
  leaving the batch's `runs/` untouched (R32).
- A denied batch leaves no manifest and does not overwrite a committed
  `plan.json` or `peer_b.json` (R31).
- `PREREGISTRATION.md` contains the scaling rule, the dilution statement, the
  settled contrasts and the claim boundary (R20, covering U2).

**Verification.** `bash research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh`
exits 0; `bash -n run-batch.sh` clean.

## Verification Contract

- `cargo test --example rt005_batch_plan` — bridge unit tests pass.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.
- `bash research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh` — all
  assertions pass, exit 0.
- Live dry run on this host as it stands (no `mika` on the non-interactive
  `PATH`, confidence agents unprovisioned, no spirit log configured):
  `run-batch.sh dry-run --out-dir <tmp>` writes a manifest with 80 runs, 20 per
  cell, and no run record.
- Gate proof against live state: with a real `mika` resolvable via `MIKA_BIN`,
  `run-batch.sh batch --out-dir <fresh tmp>` exits 3, **names the gate factor
  that failed**, and creates no run record and no manifest. The gate deliberately
  runs *before* the live preamble, so this proof needs no provisioning and a
  denial can never be confused with an unprovisioned host — which is what makes
  it discriminating rather than vacuous.
- Bounded live check: at most two real runs via `manip-check --limit 2`,
  asserting the artifact is written `coverage: partial`, that no authorization
  file appears, and that a batch presented with an authorization bound to it
  still denies. **Not performed** — the operator confirmed the zero-run posture
  on 2026-08-29, and the stubbed suite covers the same code paths. What remains
  unverified against the real binary is therefore stated in the PR rather than
  implied to have passed: that `mika ask` accepts these arguments in practice,
  and that a real `turn_usage` line lands in the captured slice.
- `bash scripts/verify-pipeline.sh` — pipeline artifacts present.

## Definition of Done

Global:
- Every acceptance criterion in mika#1890 is covered by a named test above,
  including AC4's content — a test asserts `PREREGISTRATION.md` states the
  scaling rule and the claim boundary, not merely that the file exists.
- The four guardrails are enforced by tested code paths, not by prose: the gate
  (the nine denial scenarios plus the two integrity scenarios), the
  pre-registration hash and content, the no-aggregate rule (the token-field
  scenario), and the seeded order (the order and fingerprint scenarios).
- No gate factor is carried by a check that cannot fail. Three were fixed under
  this rule: the reliability half of the manip-check is evidenced by live paired
  runs on a perturbed item (A3) rather than by a peer_b property that holds by
  construction; `coverage` counts cells that *succeeded*, so a check whose runs
  all failed cannot read as evidence (R26); and `PREREGISTRATION.md`'s absence is
  exercised by a test rather than being unreachable because the file ships beside
  the script (R20).
- No file changed outside `research/rt005-physics-pilot/orchestration/`,
  `crates/mika-agent/examples/`, and `docs/`.
- `crates/mika-agent/src/research/peer_b.rs`, the mika#1888 config assets, and
  `crates/mika-agent/Cargo.toml` are untouched — confirmed by the PR diff.
- The 80-run batch was NOT executed, and no live run was made at all: the
  operator confirmed the zero-run posture, so verification is the stubbed suite
  plus a dry run and a gate proof, both of which make zero LLM calls. What that
  leaves unverified against the real binary is stated in the PR, not implied to
  have passed.
- The README states what the gate does not stop and who it does not stop (KD1,
  KTD3) rather than overclaiming.
- No dead-end or experimental code left in the diff.
- PR opened with `Closes #1890` and `mika-platform-qa` added as reviewer.

Per unit:
- U1: the seven bridge scenarios pass, including the no-dot invariant the
  orchestrator's run-id parsing depends on; clippy and fmt clean.
- U2: `PREREGISTRATION.md` states the scaling rule, the dilution fact, the
  primary contrast, the extension-batch procedure and the claim boundary, and is
  dated before the first run; `README.md` documents the gate's limits.
- U3: every scenario above passes; `bash -n` and `shellcheck -S warning` clean;
  the dry run produces an 80-run manifest with zero LLM calls on a host with no
  `mika`, no provisioned agents and no spirit log.

## Acceptance criteria

Transcribed verbatim from mika#1890.

- [ ] Génère les 80 configurations (2×2×10×R=2), ordre randomisé seedé.
- [ ] **Hard-stop manip-check** : refuse d'exécuter le batch complet sans flag d'autorisation opérateur.
- [ ] Chaque run invoque le bon agent (confidence-high/low) contre peer_b (fiable/dégradée) et capture les logs tokens.
- [ ] Règle-de-scaling + disclaimer existence-non-magnitude écrits dans l'output.
