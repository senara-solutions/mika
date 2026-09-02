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

## Amendment 1 — 2026-08-31: how question 1 is read

**This is an amendment, and it is labelled as one.** It was written after the
manipulation check produced data and **before any batch run existed**. That is the
only window in which an analysis rule may still be fixed without being a rule
written after the data — and it is the window a manip-check exists to occupy: it
produces evidence precisely so the operator can decide how, and whether, to spend
the 80 runs. Folding this into the sections above, as if it had been there since
2026-08-29, would have been the dishonest form of the same act.

Nothing else in this document changes: not the contrasts, not the dilution, not
the claim boundary, not the scaling rule.

### The rule

Manipulation-check question 1 — *does the 0.95 / 0.55 injection move the agent's
behavior?* — is read on **`out_tokens`**, the primary outcome's own scale.

1. **Never on string equality of the outputs.** The `outputs_identical` flag stays
   recorded on the artifact, and carries **no conclusion** on question 1.
2. **On the direction within a matched order**, never on raw magnitudes compared
   across runs of different cache warmth.

### The evidence that forced it

Item `rt005-01`, fiable arm, prompts byte-identical between the two agents:

| order | high | low | cache_read (high / low) |
|---|---|---|---|
| original — high cold, low warm | 18 | **69** | 0 / 12288 |
| inverted — low cold, high warm | 3 | **38** | 12288 / 9920 |

Both agents emitted the same text, `115`. The `outputs_identical` flag therefore
reads **true** at the fiable arm — and would, read alone, license the conclusion
that the confidence knob does nothing there. On the primary outcome's own scale
the low-confidence agent deliberated roughly four times as much and arrived at the
same answer. **A flag that answers a different question than the one asked is a
false negative on a gate factor**, which is the one direction a manipulation check
must not fail in.

The inverted pair separates the knob from the cache. In the inverted order the
low-confidence agent ran with **less** cache than the high-confidence one (9920 vs
12288) and still emitted more output tokens. Had the gap been a warmth artifact,
reversing the warmth would have reversed the gap. It did not.

### Why the seeded random order is a conscious protection

The same two runs show that **magnitude is not robust to warmth while direction
is**: the ratio is 3.8x in the original order and 12.7x in the inverted one, same
sign, very different size. The second run of each order — the warmer one — emitted
less in **both** arms (high 18→3, low 69→38).

A batch executing its cells in a **fixed** order would therefore bias later cells
systematically downward. This protocol's seeded random ordering already prevents
that. It is recorded here so the protection is held knowingly rather than by luck,
and so that a future reader who is tempted to "simplify" the ordering knows what it
was buying.

### Provenance and hash lineage

Evidence: batch `rt005-manipcheck-20260831`, artifact sha256
`23fddab14041f9b3ddcefdaeea36ea200cf8c966619aa4db491e0230fd762b95`, design
fingerprint `28bdf138f1c5c2c1e754879ab766062676a5ceee30771f54226258cb3f7c3474`. The
inverted pair was run **outside** any batch (sessions `rt005-inv-*`), wrote nothing
into a batch directory, and changed neither the design nor the fingerprint.

This document's sha256 before this amendment was
`da8e6432ed1e5c455473d6a7771de4244f723b6081caefb474b7bccb36272a56` — the value
recorded in the manip-check batch's manifest. The 80-run batch will record the new
value. The two differing hashes are the lineage, not a discrepancy: any reader
comparing them lands here.

Amendment settled by the operator (samidarko seat), 2026-08-31, on evidence
produced and reported the same day. The 80 runs remain behind Vincent's go.
