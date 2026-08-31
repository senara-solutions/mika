---
issue: mika#2116
title: RT-005 Analyser Invocation Surface - Plan
type: feat
scope_repo: mika
priority: p2-normal
date: 2026-08-31
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# RT-005 Analyser Invocation Surface - Plan

## Goal Capsule

**Objective.** Anyone holding an RT-005 batch directory can produce the
pre-registered report by running one command, without writing code. Today that
requires reconstructing a harness, and a report that must be reconstructed is a
report that will eventually be reconstructed wrong.

**Means.** A `cargo run --example` entry point that loads, analyses, and prints —
and computes nothing (KTD2).

**Authority hierarchy.** `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md`
> issue ACs > this plan. The estimand is defined in exactly one place —
`mechanism_analyzer.rs` — and this work does not add a second.

**Stop conditions.**
- Stop if the entry point would compute, aggregate, filter, or reformat any
  number. That is a second definition of the estimand, which is the fork the
  module exists to forbid.
- Stop if the change would alter `load_batch`, `analyze`, or `render`. This
  ticket adds access, never behaviour.

**Execution profile.** Single repo, one new file, no `Cargo.toml` entry.

## Product Contract

### Summary

Add `crates/mika-agent/examples/rt005_analyze.rs`: three calls and a print. It
makes the pre-registered analysis reproducible by anyone, without promising API
stability the `research/` module does not have.

### Problem Frame

`mechanism_analyzer.rs` (mika#1891) exposes `load_batch`, `analyze` and
`Report::render`. **Nothing in the tree calls them.** The only file mentioning
the module is `research/mod.rs`, which declares it.

This was deliberate: the parent plan put *"any CLI subcommand"* and *"no
agent-loop wiring, no tool registration, no CLI surface"* out of scope, and that
was defensible when written.

**What made it a defect is the day the analysis was needed.** On 2026-08-31 the
80-run batch finished, the operator asked for the pre-registered report, and no
command existed to produce it. The report was produced by a **throwaway harness
outside the repo** — twelve lines chaining the three functions and printing the
render verbatim. It worked first try, which proves the library is sound: what is
missing is access, not computation.

The brick was complete by its own acceptance criteria and unreachable in
practice. The cost that day was minutes. The real cost is that an analyser
invocable only by writing code is one that will not be invoked — or will be
reimplemented wrong under pressure, and the pre-registered estimand is precisely
the thing that must never be reimplemented.

### Key Decisions

- **An example, not a CLI subcommand.** Cargo auto-discovers `examples/*.rs`, so
  it adds no `Cargo.toml` entry, no runtime surface, and no API surface, and it
  deletes with the rest of the RT-005 scaffold. `research/` describes itself as
  disposable experiment apparatus; a subcommand would give a disposable apparatus
  the appearance of a product feature, and would contradict the parent plan's
  scope note head-on. Precedent: `examples/rt005_batch_plan.rs` (brick 3/5).
  Governs R1, R4.

### Requirements

- R1. `crates/mika-agent/examples/rt005_analyze.rs` exists and takes a batch
  directory path.
- R2. It calls `load_batch`, then `analyze`, then prints `render()` **verbatim**
  to stdout.
- R3. It computes nothing: no arithmetic, no aggregation, no filtering, no
  reformatting of any value the report carries.
- R4. No `Cargo.toml` entry, no CLI subcommand, no tool registration, no
  agent-loop wiring.
- R5. `load_batch`, `analyze`, and `render` are unchanged — verified by an empty
  diff on `mechanism_analyzer.rs`.
- R6. A failure to load names the directory and the reason, and exits non-zero.
  A silent or zero-exit failure would let a caller believe an empty report is a
  result.

### Acceptance Examples

- AE1. `cargo run -p mika-agent --example rt005_analyze -- ~/.mika/rt005/rt005-20260831`
  prints the same report text that the throwaway harness produced on 2026-08-31.
  Covers R1, R2.
- AE2. Given a path that is not a batch directory, it exits non-zero and the
  message names the path and the reason. Covers R6.

### Sources

- Issue: `senara-solutions/mika#2116`; parent `mika#1891` (closed)
- `crates/mika-agent/src/research/mechanism_analyzer.rs:292` `load_batch`,
  `:543` `analyze`, `:696` `Report::render`
- Precedent and form to match: `crates/mika-agent/examples/rt005_batch_plan.rs`
- Parent scope note: `docs/plans/2026-08-29-005-feat-1891-research-offline-mechanism-analyzer-plan.md`
- Execution evidence of the throwaway harness: `~/.mika/rt005/rt005-20260831-analysis/`
  (report + `PROVENANCE.md`), a sibling of the batch directory

## Planning Contract

### Key Technical Decisions

- KTD1. **Form: example.** Instantiates the Key Decision above; see its rationale.
  Governs R1, R4.
- KTD2. **The entry point computes nothing, and this is guarded by effect, not by
  intent.** Arithmetic in the runner would be a second definition of the estimand
  — two places that could silently disagree about what RT-005 measures. The guard
  is **not** a source-level lint: linting the example's source checks intent, is
  fragile (negative literals, computed indices), and costs more to maintain than
  the disposable file it protects. The guard is a **frozen fixture** (U2): a
  committed batch and the exact report `render()` produces for it. Any arithmetic
  added anywhere on the path changes the output and turns the test red. The
  docstring states the rule; the fixture enforces it. Governs R3.
- KTD3. **Argument handling stays hand-rolled.** One positional path. Adding a
  parser crate to an example that takes one argument would be the first step of
  the surface this ticket declines to build.

### Assumptions

- `load_batch` accepts a batch directory as produced by `run-batch.sh` without
  modification. Verified by AE1 running against the real `rt005-20260831`.

### Sequencing

U1 (the example) then U2 (the frozen fixture) then U3 (the doc pointer). U2 after
U1 because it asserts against the path U1 exercises.

## Implementation Units

### U1. `rt005_analyze.rs`

**Goal.** Three calls and a print.

**Requirements.** R1, R2, R3, R4, R6.

**Files.** `crates/mika-agent/examples/rt005_analyze.rs` (new).

**Approach.** Module docstring in the shape of `rt005_batch_plan.rs`, stating what
it does, why it is an example and not a subcommand, and — load-bearing — **that it
computes nothing and must not start**. Read one positional path. `load_batch`,
`analyze`, print `render()`. On a load error, print the path and the reason to
stderr and exit non-zero.

**Test scenarios.** AE1 against the real batch. AE2 against a non-batch path.
**Negative control:** run it against an empty directory and confirm it fails
loudly rather than printing an empty report — an analyser that renders nothing on
no data is indistinguishable from one that renders nothing on broken data.

**Verification.** `cargo run -p mika-agent --example rt005_analyze -- <dir>`;
`cargo build -p mika-agent --all-targets`.

### U2. Freeze a fixture batch so the effect-guard runs in CI

**Goal.** The guard that actually protects the estimand is the one that compares
the produced report against a known-good one. Make it runnable without a machine
that happens to hold `~/.mika/rt005/`.

**Requirements.** R3, R5.

**Files.** `crates/mika-agent/tests/fixtures/rt005-analyze/` (new: a small batch
directory and its expected report); a test beside the existing research tests.

**Approach.** Commit a **small synthetic batch** — a handful of runs, enough for
`analyze` to produce every section of the report — together with the report text
`render()` produces for it. The test runs the same three calls the example runs
and asserts the output matches the frozen text byte for byte.

**Why this replaces a source-level no-arithmetic lint.** A lint on the example's
source checks *intent*, is fragile (a negative literal, a computed index), and
costs more to maintain than the disposable file it guards. The frozen-fixture
test checks *effect*: any arithmetic added anywhere on the path — in the example
or in the analyser — changes the output and turns the test red. It is robust,
cheap, and it is the guard the Verification Contract already leaned on; this unit
only makes it reproducible off the operator's machine.

**Test scenarios.** The frozen report is reproduced exactly. **Non-vacuity proof:**
add `let _ = 1 + 1;` — no; that would not change output. Instead alter the example
so it prints a value it computed itself, confirm the test goes red, revert, and
record the demonstration in the PR body. A guard that cannot be seen to refuse is
not known to work.

**Verification.** `cargo test -p mika-agent rt005_analyze`.

### U3. Point the ticket's readers at it

**Goal.** The next person needing the report finds the command.

**Requirements.** R1.

**Files.** `research/rt005-physics-pilot/orchestration/README.md`.

**Approach.** One line under the batch instructions: the command that produces the
pre-registered report from a batch directory. No duplication of the estimand's
description — a pointer, not a second explanation.

**Test scenarios.** None.

**Verification.** The command in the README runs as written.

## Verification Contract

- `cargo build -p mika-agent --all-targets` — the example is an `--all-targets`
  build target from now on; that is the accepted price.
- `cargo clippy --all-targets -- -D warnings`
- `cargo test -p mika-agent rt005_analyze` — including U2's non-vacuity proof.
- `cargo test -p mika-agent rt005_analyze` — the frozen-fixture effect guard
  (U2). This is the guard that runs in CI, and the only one that does.
- AE1 reproduced **locally** against `~/.mika/rt005/rt005-20260831`, and its
  output compared to the report already stored in
  `~/.mika/rt005/rt005-20260831-analysis/`. **They must match.** A new entry point
  producing a different report than the one already delivered to the operator
  would mean one of the two is wrong, and the ticket would stop there. This check
  cannot run in CI — it depends on a directory outside the repo — which is why U2
  exists: without a committed fixture, the only real guard would live on one
  machine.
- `git diff main -- crates/mika-agent/src/research/mechanism_analyzer.rs` is
  **empty** (R5).

## Definition of Done

**Global.**
- R1–R6 satisfied, each traced to a landed unit.
- The report produced by the example matches the one already stored for
  `rt005-20260831`, byte for byte.
- `mechanism_analyzer.rs` diff empty.
- No `Cargo.toml` entry added; no CLI subcommand; no tool registration.
- U1's negative control (empty directory fails loudly) and U2's non-vacuity proof
  (a self-computed value turns the fixture test red) both demonstrated in the PR
  body, not merely claimed.
- The frozen fixture runs in CI. No guard on this path depends on a directory
  outside the repository.

**Per unit.** Each unit's Verification passes.
