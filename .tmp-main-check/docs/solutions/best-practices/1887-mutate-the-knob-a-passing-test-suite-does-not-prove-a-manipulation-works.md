---
module: research
tags: [experiment-scaffold, manipulation-check, mutation-testing, determinism, seeded-rng, reproducibility, rt-005, peer_b, dependency-pinning]
problem_type: silent-null-result
category: best-practices
---

# Mutate the knob: a green test suite does not prove an experimental manipulation works

## Problem (mika#1887)

RT-005 estimates the interaction of *injected confidence* with *actual peer
reliability* on planning-token count. `research::peer_b` is the reliability arm:
a `Reliability` knob switches a stub solver between answering correctly and
answering a seeded subset wrongly.

A knob like this has a failure mode that ordinary testing does not reach. If the
degraded arm quietly fails to degrade — or degrades in a way the consumer can
route around — every downstream run still completes, still logs, still produces
a clean dataset. The batch measures noise and reports a null interaction. Nothing
errors. The null is indistinguishable from a real null, and by the time the
analysis runs, the 80 runs are spent.

The first implementation of `peer_b` passed 13 tests, `clippy -D warnings`, and
`fmt`. It still shipped two paths where the manipulation was inert or
recoverable:

- **The inert arm.** Perturbation count is `n * 2 / 6`. For any fixture below 3
  items that is `0`, so `PeerB::with_fixture(small, Degradee, seed)` returned
  `Ok` for an instance that answered everything correctly while reporting itself
  degraded. The production fixture has 10 items, so no test touched it — but
  `with_fixture` is public and the orchestration script may pass a subset.
- **The recoverable manipulation.** `peer_b_solve(item, k)` filled positions
  `1..k` with distractors drawn from other fixture answers. For a perturbed item
  that pool still contained the item's own ground truth, so at any `k > 1` peer B
  handed back the correct answer beside the wrong one.

Neither is visible from "the tests pass". Both are visible in one pass from
"break the knob and see which test notices".

## Solution

**Mutation-check every manipulation knob before claiming it works.** Deliberately
break the mechanism the experiment depends on, run the suite, and record which
test falls. A mutation no test catches is a hole in the manipulation, not a
missing unit test — it means the experiment could run to completion measuring
nothing.

For `peer_b` the five mutations and their catchers:

| Mutation | Caught by |
|---|---|
| `Degradee` perturbs nothing | `degradee_perturbs_exactly_three_of_ten`, `degradee_perturbs_exactly_two_of_six`, `different_seed_selects_different_perturbed_set` |
| Perturbed answer is a corrupted string, not a real answer | `perturbed_answers_are_well_formed_fixture_answers` |
| Seed ignored (fixed stream) | `different_seed_selects_different_perturbed_set` |
| Fixture-too-small guard removed | `fixture_too_small_to_degrade_is_rejected` |
| Ground truth leaks back as a distractor | `distractors_never_hand_back_a_perturbed_item_truth` |

The last two mutations exist *because* the mutation pass found the bugs first.
That is the point: the exercise is diagnostic, not confirmatory.

Three design rules fell out of it, and they generalise to any experiment stub:

1. **An inert manipulation is a construction-time error, not a silent success.**
   If the knob cannot produce a distinguishable arm for the fixture it was
   handed, refuse to build. A degraded arm that degrades nothing must be
   impossible to hold, not merely unlikely.
2. **The wrong answer must be indistinguishable by shape.** Perturb by
   substituting another well-formed answer, never by corrupting characters. A
   consumer that can spot malformed output routes around the knob without
   reasoning, and the manipulation the study rests on is gone.
3. **Never hand back what you withheld.** Any secondary surface — a k-best list,
   a hint, a confidence score — must be checked for whether it re-exposes the
   truth the primary answer withheld.

## Reproducibility outlives the dependency

`peer_b` ships a ~15-line SplitMix64 rather than taking `rand`. This is not
dependency minimalism. A recorded batch must be replayable from its seed for as
long as the result is cited, and `rand` changes its generator algorithms across
major versions — a routine `cargo update` would silently re-select which items
were perturbed and make an already-published run irreproducible, with no build
error to warn anyone.

**When reproducibility is a claim in a result, pin the algorithm in-tree, not the
version in a manifest.** A version pin is a promise someone can bump; a
transcribed algorithm is a fact. The same reasoning applies to any hash,
shuffle, or sampling step whose output appears in a published artifact.

Related discipline: correctness of a stub's answers must be decidable without a
judge. `peer_b`'s fixture uses short arithmetic and string tasks graded by string
equality, so "was this answer right?" never becomes its own measurement problem.

## Cross-references

- `crates/mika-agent/src/research/peer_b.rs` — the module and its 16 tests.
- mika#1890 — the orchestration script, whose operator-held manip-check gate is
  the human-scale version of this same check.
- `feedback_verify_pipeline_passes_without_the_fix` — the general form: a test
  that passes with the fix reverted proves nothing.
