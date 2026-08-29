---
title: peer_b Internal Stub Module (RT-005 Brick 1/5) - Plan
type: feat
date: 2026-08-29
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# peer_b Internal Stub Module (RT-005 Brick 1/5) - Plan

## Goal Capsule

**Objective.** The RT-005 operator can run the 2x2 pilot and know, from the run
record alone, whether the "real reliability" arm actually differed — every
answer peer_b gave is reproducible from a seed and checkable against a known
truth, so a null interaction result can be attributed to the phenomenon rather
than to an inert knob.

**Means.** A dependency-free internal Rust module `research::peer_b` in
`mika-agent` holding a ground-truth fixture and a seeded perturbation pass
(KTD1, KTD2).

**Authority hierarchy.** Issue mika#1887 `## Scope` and `## Acceptance criteria`
are authoritative — the issue body carries no valid plan callout and no prior
plan exists. The RT-005 Round Table ratification (2026-07-28) and Vincent's
2026-08-03 decision (stub, not MCP server) bound the design. Sibling brick
mika#1890 fixes the run cardinality (2x2x10 items x R=2 = 80 runs).

**Stop conditions.** Stop at the module plus its unit tests. No MCP server, no
transport, no agent tool registration, no wiring into the agent loop, no
orchestration script (that is mika#1890). No file outside
`crates/mika-agent/src/research/` and `crates/mika-agent/src/lib.rs`.

**Execution profile.** Single unit of work, one crate, no migration, no config,
no network. `cargo test -p mika-agent research::peer_b` is the whole gate.

**Tail ownership.** PR opened against `senara-solutions/mika` with
`Closes #1887` and reviewer `mika-platform-qa`.

## Product Contract

### Summary

Add `crates/mika-agent/src/research/peer_b.rs`: a disposable research scaffold
that answers "what does peer B say about item X?" from a fixed table of items
whose correct answers are known. A `Reliability` knob chosen at construction
switches the module between returning every correct answer (`Fiable`) and
returning a deterministic, seeded subset of wrong answers (`Degradee`). Nothing
leaves the process.

### Problem Frame

RT-005 measures the interaction of *injected confidence* (brick 2/5, mika#1888,
shipped) with *actual peer reliability* (this brick) on planning-token count
(brick 4/5, mika#1889, shipped). The interaction is only estimable if the
reliability factor has two levels that are genuinely distinguishable. If
`Degradee` were to return answers that are wrong-but-unnoticeable, or wrong in a
way that varies between runs, the 80-run batch in mika#1890 would measure noise
and no downstream analysis (brick 5/5) could recover the signal. The operator's
manip-check gate in mika#1890 exists to catch exactly that failure before the
batch runs, so this module has to be legible to that check.

This brick is the last open item on the RT-005 critical path.

### Requirements

**Solving surface**

- R1. `peer_b_solve(item_id, k)` returns peer B's candidate answer for
  `item_id`, drawn from the fixture, with no I/O, no network, and no process
  spawn.
- R2. Requesting an unknown `item_id` is a recoverable error, not a panic.
- R3. `k` is the number of candidates requested; the response carries at most
  `k` candidates with peer B's committed answer first (KTD3).

**Ground-truth fixture**

- R4. The fixture holds at least 10 items. Every item carries its `id`, a
  prompt, and its one correct answer.
- R5. Every item's correctness is decidable by string equality against the
  fixture — no LLM, no judge, no tolerance window.
- R6. The fixture is exposed for scoring so mika#1890 and brick 5/5 can grade a
  run's answers against truth without re-deriving them.

**Reliability knob**

- R7. `Reliability` has exactly two levels, `Fiable` and `Degradee`, fixed at
  construction and immutable afterwards.
- R8. Under `Fiable`, every item's answer equals its ground truth. Zero
  perturbations.
- R9. Under `Degradee`, the perturbed item count is `n * 2 / 6` in integer
  division, where `n` is the fixture size (KTD4). Which items are perturbed is
  drawn from the seeded perturbation RNG.
- R10. A perturbed answer is wrong (never equal to ground truth) and is drawn
  from the same surface form as the correct answer, so a consumer cannot
  distinguish it by shape alone.

**Determinism**

- R11. Two instances built with the same `(Reliability, seed)` produce
  byte-identical answers for every item, in any call order, across processes and
  across releases (KTD2).
- R12. Different seeds under `Degradee` select different perturbed item sets, so
  the operator can re-randomise without editing the fixture.
- R13. The module reports which items it perturbed, so a run record can carry
  the realised reliability rather than the nominal one.

### Key Decisions

- **Internal module, not an MCP server.** Vincent, 2026-08-03: stub/scaffold,
  reversible, per coherence's YAGNI. The transport is incident to the estimand.
  Governs R1.
- **`2/6` read as a rate, not a count.** The issue's `## Scope` says the fixture
  holds >=10 items and its acceptance criteria say `Degradee` perturbs "exactly
  2/6". mika#1890 fixes the design at 10 items, all used. The `6` is stale from
  an earlier protocol draft. Treating `2/6` as the one-third *rate* honours both
  statements: a 6-item fixture yields exactly 2, and the real 10-item fixture
  yields 3. Governs R9. See KTD4.
- **Disposable scaffold.** No persistence, no config surface, no registration in
  the tool registry, no public API stability promise. Governs the Scope
  Boundaries below.

### Acceptance Examples

- AE1. `PeerB::new(Reliability::Fiable, 42)` then `peer_b_solve(id, 1)` for all
  10 fixture ids returns 10 answers, each equal to that item's ground truth.
  Covers R8.
- AE2. `PeerB::new(Reliability::Degradee, 42)` over the 10-item fixture returns
  exactly 3 answers that differ from ground truth and 7 that match. Covers R9.
- AE3. A 6-item fixture under `Degradee` returns exactly 2 wrong answers — the
  issue's literal criterion. Covers R9, KTD4.
- AE4. Two `PeerB::new(Reliability::Degradee, 42)` instances agree on the
  perturbed id set and on every answer string. A third built with seed `43`
  selects a different id set. Covers R11, R12.
- AE5. `peer_b_solve("no-such-item", 1)` returns `Err`, and the process does not
  panic. Covers R2.

### Scope Boundaries

In scope: the module, its fixture, its tests, and one `pub mod research;` line
in `crates/mika-agent/src/lib.rs`.

Out of scope and explicitly not built: an MCP server or any transport; a builtin
tool or agent-loop callsite; a CLI subcommand; DB persistence; a config key; the
orchestration script and manip-check gate (mika#1890); the offline analyser
(mika#1891); any change to the two confidence agent configs (mika#1888,
shipped).

### Dependencies

Upstream: none — this brick is wave 1 and independent. Downstream: mika#1890 is
blocked on this and on nothing else.

## Planning Contract

### Key Technical Decisions

- KTD1. **Place the module at `crates/mika-agent/src/research/peer_b.rs` behind
  a new `research` module.** `mika-agent` already groups small bounded
  subsystems this way (`calibration/`, `evidence/`, `perimeter/`), so a
  `research/` namespace signals "experiment scaffolding, not product surface"
  and gives brick 5/5 a home without a second structural decision later.
  Governs R1.

- KTD2. **Ship a self-contained SplitMix64 PRNG rather than taking the `rand`
  crate.** `mika-agent` does not currently depend on `rand`, and `rand` has
  changed its generator algorithms across major versions — a future dependency
  bump would silently change which items get perturbed and break reproduction of
  an already-recorded 80-run batch. Reproducibility is a protocol requirement
  here (R11), not a convenience. SplitMix64 is about 15 lines, has no state
  beyond a `u64`, and is fixed forever once written.
  (session-settled: user-directed — chosen over the `rand` crate: an algorithm
  change on a dependency bump would invalidate recorded run reproduction.)
  Governs R11, R12.

- KTD3. **`k` is the number of candidates requested; the response returns peer
  B's committed answer first.** The issue fixes the signature
  `peer_b_solve(item_id, k)` but its prose says "returns a candidate solution".
  `k` in a `solve(item, k)` signature conventionally means k-best. Returning a
  `Vec` whose head is the committed answer satisfies both readings: the protocol
  calls it with `k = 1` and reads one answer, while the signature stays as
  ratified. Distractors after the head are deterministic and never equal to the
  head. Governs R3.

- KTD4. **Perturbation count is `n * 2 / 6` in integer division.** Carries the
  ratified one-third degradation rate to any fixture size while making the
  issue's literal "exactly 2/6" criterion true at `n = 6`. A single named
  constant pair (`DEGRADED_NUM = 2`, `DEGRADED_DEN = 6`) holds the rate so a
  future protocol revision is a one-line change. Governs R9.

- KTD5. **Perturb by substituting another fixture item's answer, not by mutating
  characters.** A character-level corruption ("42" -> "4z") is detectable by
  shape and would let a consumer route around the knob without reasoning, which
  would collapse the manipulation the experiment depends on (R10). Borrowing a
  sibling item's well-formed answer keeps every response plausible. The
  substitution source is chosen by the same seeded RNG and is checked against
  the item's own truth so a perturbed answer is never accidentally correct.
  Governs R10.

- KTD6. **Perturbation is decided once, at construction, over the whole
  fixture.** Deciding per call would make the answer depend on call order and
  break R11. Building the perturbed id set in `PeerB::new` makes determinism
  structural rather than a property the caller has to preserve, and makes R13
  (report the realised perturbed set) a field read rather than an accumulation.
  Governs R11, R13.

### High-Level Technical Design

```
PeerB::new(reliability, seed)
  |
  +-- fixture: &'static [Item]            (id, prompt, truth)
  +-- SplitMix64(seed)
  |     |
  |     +-- Fiable   -> perturbed = {}                       (R8)
  |     +-- Degradee -> pick n*2/6 distinct indices          (R9, KTD4)
  |                     for each, pick a donor index != self (KTD5)
  |
  +-- answers: Vec<String>   (truth, or donor's truth if perturbed)
  +-- perturbed_ids: Vec<&'static str>                       (R13)

peer_b_solve(&self, item_id, k) -> Result<PeerBResponse>
  -> lookup index by id, else Err                            (R2)
  -> candidates[0] = answers[index]                          (R3)
  -> candidates[1..k] = deterministic distractors            (KTD3)
```

The struct owns all state; there is no interior mutability, no async, and no
`&mut self` method. `peer_b_solve` is a pure read.

### Assumptions

- The fixture items are small, objectively-checkable tasks (short arithmetic and
  string transformations) so R5 holds without a judge. RT-005 measures the
  agent's *planning* around a peer answer, not the difficulty of the item, so
  item difficulty is not a design variable here.
- `mika-platform-qa` reviews the PR; the operator holds the manip-check gate in
  mika#1890.

### Risks & Dependencies

- **The knob could still be inert in practice.** This module guarantees that
  degraded answers are wrong and reproducible; it cannot guarantee an agent
  notices. That is what mika#1890's manip-check gate tests, and R13 exists so
  that check has a ground-truth diff to work from.
- **`~150 lines` is a budget, not a target.** The fixture table plus tests will
  exceed it. Keep the *logic* at that scale; the data and tests are not the part
  the guardrail is aimed at.

## Implementation Units

### U1. `research::peer_b` module, fixture, seeded perturbation, and tests

**Goal.** Land the whole brick in one unit — the module is small enough that
splitting it would create dependencies between units that all touch the same
file.

**Requirements.** R1-R13.

**Files.**
- `crates/mika-agent/src/research/mod.rs` (new) — module docstring stating this
  is RT-005 experiment scaffolding, disposable, no product surface; `pub mod
  peer_b;`.
- `crates/mika-agent/src/research/peer_b.rs` (new) — the module and its
  `#[cfg(test)] mod tests`.
- `crates/mika-agent/src/lib.rs` — add `pub mod research;` in alphabetical
  position (between `prompt` and `rewind`).

**Approach.**

1. `Item { id: &'static str, prompt: &'static str, truth: &'static str }` and a
   `const FIXTURE: &[Item]` of 10 items (R4). Ids are stable slugs
   (`rt005-01` ... `rt005-10`) because mika#1890 will name them in run records.
2. `SplitMix64 { state: u64 }` with `next_u64` and a `next_below(n)` helper
   (KTD2). Note in a comment that the algorithm is pinned for reproducibility
   and must not be swapped.
3. `Reliability { Fiable, Degradee }`, `Copy`, `PartialEq`, and a `FromStr` or
   `from_label` accepting `"fiable"` / `"degradee"` so mika#1890 can pass the
   knob as a string without re-implementing the mapping.
4. `PeerB::new(reliability, seed)` and `PeerB::with_fixture(fixture,
   reliability, seed)` — the second is what makes AE3's 6-item case testable
   without a second production fixture.
5. In `new`: under `Fiable`, `answers` is a clone of every truth and
   `perturbed_ids` is empty (R8). Under `Degradee`, compute `count = len *
   DEGRADED_NUM / DEGRADED_DEN` (KTD4), draw `count` distinct indices with the
   RNG, and for each draw a donor index `!= self` whose truth differs from the
   item's own truth, then substitute (KTD5). Record `perturbed_ids` (R13).
6. `peer_b_solve(&self, item_id: &str, k: usize) -> anyhow::Result<PeerBResponse>`
   — `PeerBResponse { item_id, candidates: Vec<String> }`. Unknown id is
   `anyhow::bail!` (R2). `k == 0` is clamped to 1. Distractors for `k > 1` are
   drawn from other items' truths, deduped against the head (R3, KTD3).
7. Scoring surface: `ground_truth(item_id) -> Option<&'static str>`,
   `fixture() -> &'static [Item]`, `perturbed_ids() -> &[&'static str]`,
   `reliability()`, `seed()` (R6, R13).

**Test Scenarios.**
- `fiable_returns_ground_truth_for_every_item` — AE1, R8.
- `degradee_perturbs_exactly_three_of_ten` — AE2, R9.
- `degradee_perturbs_exactly_two_of_six` — AE3, the issue's literal criterion,
  via `with_fixture` on a 6-item table.
- `same_seed_same_answers_and_same_perturbed_set` — AE4, R11.
- `different_seed_selects_different_perturbed_set` — AE4, R12.
- `perturbed_answers_are_never_ground_truth` — R10, over several seeds.
- `perturbed_answers_are_well_formed_fixture_answers` — KTD5: every perturbed
  answer is some other item's truth, not a corrupted string.
- `unknown_item_id_is_an_error_not_a_panic` — AE5, R2.
- `k_greater_than_one_returns_distinct_candidates_head_first` — R3, KTD3.
- `call_order_does_not_change_answers` — R11: solve in reverse order, compare.

**Verification.** `cargo test -p mika-agent research::peer_b`, then
`cargo clippy -p mika-agent --all-targets -- -D warnings` and `cargo fmt --check`.

**Dependencies.** None.

## Verification Contract

- `cargo test -p mika-agent research::peer_b` — all unit tests pass.
- `cargo test -p mika-agent` — no regression in the existing suite.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.
- Structural check for the no-egress criterion: `grep -rnE
  'reqwest|tokio::net|std::process|std::net|std::fs' crates/mika-agent/src/research/`
  returns nothing. This is the mechanical form of R1's "no egress, no server"
  criterion — assert it rather than assert it in prose.
- `bash scripts/verify-pipeline.sh` — pipeline artifacts present.

## Definition of Done

Global:
- Every acceptance criterion in mika#1887 is covered by a named test above.
- No file changed outside `crates/mika-agent/src/research/` and the single
  `pub mod research;` line in `crates/mika-agent/src/lib.rs`.
- No new entry in `Cargo.toml` — the `rand` dependency is deliberately not
  taken (KTD2).
- The module docstring names the ticket, the Round Table date, the disposable
  status, and the `2/6`-as-rate reading, so a reader of the file alone does not
  have to reconstruct the tension from the issue thread.
- No dead-end or experimental code left in the diff.
- PR opened with `Closes #1887`, the `2/6` resolution stated in the body, and
  `mika-platform-qa` added as reviewer.

Per unit:
- U1: the ten test scenarios above pass; clippy and fmt clean; the no-egress
  grep returns nothing.

## Acceptance criteria

Transcribed verbatim from mika#1887.

- [ ] `peer_b_solve` appelable en interne (skill/module), pas d'egress, pas de serveur.
- [ ] Knob RELIABILITY bascule fiable↔dégradée à l'init.
- [ ] Fixture ≥10 items avec ground-truth.
- [ ] RNG de perturbation seedé (2/6 perturbés en mode dégradé).
- [ ] Tests unitaires : mode fiable = 0 perturbation, mode dégradé = exactement 2/6.
