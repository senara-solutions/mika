# RT-005 physics pilot — pre-registration

**Written 2026-08-29, before the first run of any batch.** Nothing in this
document was written after seeing data. `run-batch.sh` requires this file to
exist and records its SHA-256 in every batch manifest before the first run, so a
later reader can verify that the rules below predate the data they govern.

Ticket: mika#1890 (brick 3/5). Protocol: RT-005, ratified by Round Table
2026-07-28.

## Design

Factorial 2x2, fully crossed:

| Factor | Levels | Realised by |
|---|---|---|
| Injected confidence | 0.95 / 0.55 | `mika-dev-confidence-{high,low}` (mika#1888), out-of-band in `soul.md` |
| Peer reliability | fiable / degradee | `research::peer_b` `Reliability` knob (mika#1887) |

10 fixture items x 2 replicates x 4 cells = **80 runs**, 20 per cell, executed
one at a time in a seeded random order.

**Primary outcome: planning tokens**, derived offline by mika#1891 from the raw
`turn_usage` records (mika#1889). This orchestrator computes nothing; it records.

## The dilution, stated before the data

`peer_b` perturbs `n * 2 / 6` items. On the ten-item fixture that is **3 items**.
A run's prompt carries only the item and peer_b's answer, so for the other 7
items the prompt is **byte-identical across the two reliability arms**.

Therefore, of the 20 runs in each degraded cell:

- **6 carry a wrong peer answer** (3 items x 2 replicates)
- **14 are input-identical** to their fiable counterparts

The design is balanced on the *label*, not on the *manipulation*. This is a
property of the ratified protocol, not a defect introduced here, and changing
peer_b's rate would mean changing a merged sibling brick. It is recorded because
an artifact reporting "20 per cell" without it invites a reader to infer 20
manipulated observations.

Every run record therefore carries `perturbed: true|false`, read from peer_b's
realised `perturbed_ids` — a design fact, not a computed statistic.

## Analysed contrast — fixed here, before the data

- **Primary contrast: the labelled arm** (fiable vs degradee, 20 vs 20 per
  confidence level). This is the assigned-treatment contrast and it is what the
  2x2 randomisation licenses.
- **Pre-specified secondary contrast: the realised perturbation** (the 6
  manipulated runs per degraded cell against their 6 input-identical fiable
  counterparts, with the remaining 14 pairs as within-design controls).

Both are named now so that choosing between them after seeing the data is not
available. Reporting the secondary contrast as if it had been primary, or vice
versa, is a protocol violation regardless of which one is more favourable.

**Settled by the operator, 2026-08-29, before any data existed.** The primary
contrast is the labelled arm — this protocol's intention-to-treat estimand — and
the estimand is not changed after the fact. The realised perturbation is a
**pre-specified secondary contrast**, recorded here and in every batch manifest
under that name. Both contrasts are reported:

> Reporting only one is a protocol violation, whichever one it is.

## Scaling rule — fixed here, before the data

If the batch has insufficient power to speak to the existence of an
interaction:

1. **First, replicate: R=2 → R=3.** Trigger: the batch completes with fewer than
   72 of 80 runs carrying usable `turn_usage` records, or the primary contrast's
   direction is not stable across the two existing replicates. Cost: 40 further
   runs.
2. **Then, and only if replication did not resolve it, extend items: 10 → 15.**
   Trigger: R=3 completed and the primary contrast remains unresolved. Note that
   extending the fixture changes `perturbed_count` from 3 to 5, which changes the
   dilution — the new ratio must be restated here before that batch runs.

No third escalation is pre-registered. If both steps are exhausted, the result
is reported as unresolved rather than pursued further.

**How a scale-up is executed.** A scale-up is a **new batch**: a new output
directory, a new batch id, its own manip-check and authorization, and
`--extends-batch <prior id>` so the manifest records the lineage in its
`extends_batch` field. The prior
batch's directory is never rewritten. This matters because the authorization
binds to the replicate count: changing R in place would either re-spend the 80
runs already bought or leave a manifest claiming an order those runs were not
executed in.

Stated honestly, because a pre-registration that promises machinery it does not
have is worth as little as a rule written after the data: `--replicates 3` plans
all 120 runs in a fresh seeded order, **not** only the 40 added ones. Reusing
the 80 already bought is a manual step — copy the prior batch's successful
`runs/*.json` into the new directory before authorizing. The orchestrator's
resume logic then skips exactly those whose design fingerprint still matches,
and the manifest's order still describes the full 120-run design the analysis
reads.

## Claim boundary — existence, not magnitude

**The claim this pilot can support is that the interaction EXISTS.**
**It cannot support a claim about how large it is.**

The reliability knob is synthetic: peer_b is a stub answering a fixture of
deliberately easy items, chosen so correctness is decidable by string equality.
An effect size measured against a synthetic peer on trivial items does not
transfer to a real collaborator on real work — external validity is bounded by
construction. Any number derived from this batch is a property of this
apparatus, not an estimate of a quantity in the world.

Consequences that are wired into the code rather than left to this document:
`run-batch.sh` computes no statistic, never parses a token count, and copies
`turn_usage` lines verbatim; and this boundary is written into the header of the
manifest, the manip-check artifact, and every run record.

## Manipulation check — what it must show, and what it cannot

Before the 80 runs may be spent, a manip-check must produce evidence on two
questions:

1. Does the 0.95 / 0.55 injection move the agent's behavior?
2. Does the reliability knob produce something the agent distinguishes?

Both are answered by **live paired runs**. Neither may be answered by a
structural property of `peer_b`: `PeerB::with_fixture` refuses to build a
degraded instance that perturbs nothing, so "the degraded arm's answers differ
from the fiable arm's" is true by construction and can never register a failure.
A gate factor that cannot fail carries no information.

The reliability pair is run on an item peer_b **actually perturbs**. On any of
the other 7 items the two arms are input-identical, so pairing there would
compare a run with itself.

The manip-check also reports the **realised perturbation rate per cell** — how
many of each cell's runs actually carry a wrong peer answer. That is the
dilution above, made visible on the artifact the operator reads before
authorizing, rather than left to this document. It is a design fact counted from
peer_b's realised `perturbed_ids`, not a measurement.

The manip-check produces evidence. It does not decide. The operator decides, and
records that decision in the authorization's `verdict` field.
