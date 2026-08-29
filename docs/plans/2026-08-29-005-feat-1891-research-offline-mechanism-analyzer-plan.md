---
title: Offline Mechanism Analyzer (RT-005 Brick 5/5) - Plan
type: feat
date: 2026-08-29
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Offline Mechanism Analyzer (RT-005 Brick 5/5) - Plan

## Goal Capsule

**Objective.** After the RT-005 2x2 batch has run, the operator can answer one
question from the recorded logs alone — *does the sign of the reliability effect
on planning tokens flip between the high-confidence and low-confidence arms?* —
and can hand that answer to a reader who will not mistake it for a claim about
how large the effect is.

**Means.** A dependency-free, strictly offline Rust module
`research::mechanism_analyzer` in `mika-agent` that parses the mika#1889
`turn_usage` JSON log stream, computes a single pre-registered interaction
contrast on planning tokens, and renders a report whose external-validity
disclaimer is its first line (KTD1, KTD2, KTD4).

**Authority hierarchy.** Issue mika#1891 `## Scope` and `## Acceptance criteria`
are authoritative. Above them sit Prime's three ratified hard guardrails
(restated as R1/R2/R3 below) — where the ticket wording and a guardrail could be
read as conflicting, the guardrail wins. Below them: the mika#1889 plan's R4/R5
and D1/D3, which fix what the log does and does not carry.

**Stop conditions.** Stop and escalate rather than improvise if: (a) the
`turn_usage` emitter in `crates/mika-agent/src/agent_loop/mod.rs` no longer
matches the field list in KTD1; (b) satisfying an acceptance criterion would
require a second primary test, a second primary metric, or moving a covariate
into the estimand; (c) the work would require running any part of the RT-005
protocol to produce data.

**Execution profile.** Single unit, one new file plus a one-line `mod`
declaration. No behaviour change anywhere in the agent loop.

**Tail ownership.** The analyzer's *input contract* (§ Dependencies) is
consumed by mika#1890 (brick 3/5), which is not yet built. This plan defines
that contract; brick 3/5 conforms to it.

## Product Contract

### Summary

Bricks 1/5 (`research::peer_b`, the reliability knob), 2/5
(`mika-dev-confidence-{high,low}`, the confidence knob) and 4/5 (per-turn
`turn_usage` token logging) have shipped. This brick is the read side: it turns
the log stream from the 80-run batch into one pre-registered existence claim.

The whole point of RT-005 is a single interaction — *injected confidence x real
reliability* on planning tokens. Everything in this module exists to compute
that one number's **sign behaviour** and to make it structurally hard for a
later contributor to widen the claim.

### Problem Frame

Three failure modes are what this module is defending against, and each one has
a named guardrail:

1. **Estimand sprawl.** A "mechanism analyzer" naturally grows into a dashboard:
   four metrics, a table of p-values, a per-item breakdown. Every extra test is
   a garden fork in the analysis path, and RT-005 pre-registered exactly one.
2. **Covariate leakage.** Turns, handshakes and recalculations are useful
   description and terrible outcomes — they are partly *definitional* (a turn is
   a turn because the loop says so). If they can be summed into the primary
   metric by accident, the estimand is no longer the estimand. A comment saying
   "do not do this" is not a defence.
3. **Magnitude creep.** The reliability knob is synthetic (`peer_b` returns
   another fixture item's answer, seeded, at a fixed rate). Any effect size read
   off this pilot is an artifact of the knob's calibration, not of the world. The
   defensible claim is existence and direction.

The mika#1889 log was deliberately built to *not* answer the planning-vs-
execution question (its D1/R5: raw dimensions only, "un thermomètre gradué en
définition" is the failure it avoids). Defining that boundary is this brick's
job, and defining it means pre-registering it in code, once, visibly.

### Requirements

- **R1 — Single primary estimand (Prime hard guardrail 1).** Exactly one
  interaction contrast, on exactly one metric: planning tokens. No family of
  tests, no dashboard, no second primary outcome.
- **R2 — Structural covariate separation (Prime hard guardrail 2).** Turns,
  handshakes and recalculations are descriptive covariates reported in their own
  section. The separation is enforced by the **type system**, not by convention:
  it must be impossible to compile a program that feeds a covariate into the
  primary estimand.
- **R3 — Existence, not magnitude (Prime hard guardrail 3).** The report's claim
  is the *sign* of the interaction and whether the simple effects flip sign. The
  bounded-external-validity disclaimer is the report's **first content**, not a
  footnote.
- **R4 — Parse the mika#1889 format as shipped.** Consume the JSON-lines
  `turn_usage` stream exactly as `logging.rs` writes it
  (`fmt::layer().json().flatten_event(true)`), tolerating unrelated log lines in
  the same file.
- **R5 — Pre-registered planning-token definition.** One definition, stated in
  the module docs, justified, and covered by tests. No runtime switch, no
  alternate definition kept "for comparison" — an analyst-selectable boundary is
  a garden fork.
- **R6 — Continuation turns count (mika#1889 R4).** The `step = u32::MAX`
  continuation turn consumes planning-class tokens; excluding it would undercount
  the primary outcome.
- **R7 — Strictly offline.** No network, no subprocess, no filesystem writes, no
  ability to launch any part of the protocol. Input is bytes the caller already
  has.
- **R8 — No product surface.** Consistent with the `research` module doctrine:
  disposable apparatus, not registered as a tool, not wired into the agent loop.

### Key Decisions

- The confidence factor is read from the **log** (`agent_id`), the reliability
  factor from the **manifest**. Each factor is read from its own authority
  (KTD3).
- The estimand uses `output_tokens` only, which is what makes it immune to the
  cross-provider `input_tokens` asymmetry (KTD2).
- Runs whose `session_id` is absent from the manifest are **dropped**, never
  guessed into a cell.

### Acceptance Examples

**A. Sign flip present.** High-confidence arm spends fewer planning tokens under
degraded reliability than under reliable; low-confidence arm spends more. Simple
effects have opposite signs → verdict `SignFlip`.

**B. Same direction.** Both arms move the same way; the interaction contrast is
non-zero but no sign flip → verdict `SameDirection`. The report does not present
the contrast's size as the finding.

**C. Missing cell.** One of the four cells has no runs → verdict `Degenerate`,
no division by zero, no partial claim.

**D. Mixed log file.** The input contains `system_prompt_assembled` events and
non-JSON noise interleaved with `turn_usage` events → only `turn_usage` events
are consumed, non-JSON lines are skipped, no error.

### Scope Boundaries

**In scope:** one new module file plus its `mod` line in
`crates/mika-agent/src/research/mod.rs`, and that module's unit tests.

**Out of scope:** the orchestration script (mika#1890), any CLI subcommand or
binary, any change to the `turn_usage` emitter, any change outside
`crates/mika-agent/src/research/`, statistical inference machinery (p-values,
confidence intervals, bootstrap) — the pilot's claim is a sign, and inference
machinery would invite exactly the magnitude reading R3 forbids.

### Dependencies

**Upstream, shipped:** mika#1889 (log format), mika#1887 (`research::peer_b`),
mika#1888 (confidence agent ids).

**Downstream — the input contract this plan fixes for mika#1890.** The analyzer
takes two inputs:

1. The `turn_usage` JSON-lines stream (any reader; brick 3/5 will point it at the
   batch's log file).
2. A **run manifest**: for each of the 80 runs, `session_id -> reliability arm`.
   Reliability is not observable in the log — `peer_b`'s knob leaves no trace in
   `turn_usage` — so brick 3/5 must record it. Confidence is *not* required in
   the manifest: it is recovered from `agent_id`, so the two factors cannot
   silently disagree.

## Planning Contract

### Key Technical Decisions

**KTD1 — Parse the flattened JSON event, keyed on `event == "turn_usage"`.**
`logging.rs` uses `fmt::layer().json().flatten_event(true)`, so each event is one
JSON object with the tracing fields at the root alongside `timestamp`, `level`
and `target`. The analyzer deserializes only the fields it needs — `agent_id`,
`session_id`, `step`, `output_tokens`, `tool_use_in_turn` — and ignores the rest.
Lines that are not JSON, or whose `event` is not `turn_usage`, are skipped
silently: an operator's log file will contain `system_prompt_assembled` events
and other noise, and a hard failure there would make the analyzer unusable
against real captures. Rejected: reading the `llm_calls` DB table instead — R2/D2
of mika#1889 made the log the measurement channel precisely because the DB write
is gated by `MIKA_STORE_LLM_CALLS`.

**KTD2 — Pre-registered planning-token definition (R5).**

> **planning tokens of a run = the sum of `output_tokens` over that run's turns
> where `tool_use_in_turn == false`.**

Three parts, each load-bearing:

- **`output_tokens`, not `input_tokens`.** Output tokens are what the agent
  *produced* — deliberation it authored. Input tokens are context handed to it,
  dominated by the system prompt and history, and would mostly measure prompt
  size. This choice also *dissolves* the cross-provider asymmetry documented in
  `docs/solutions/best-practices/cross-provider-input-tokens-cache-inclusion-asymmetry-2026-08-20.md`:
  the Anthropic-vs-OpenAI-compat disagreement is entirely about what
  `input_tokens` includes, and the estimand never reads that field. No provider
  normalization is needed, and none is implemented — adding an `input_tokens`
  term later would silently reintroduce the miscount, so a test pins the
  exclusion.
- **`tool_use_in_turn == false`.** This is the one mechanical discriminator
  mika#1889 D3 provided for exactly this purpose. A turn that emits no tool call
  is deliberation without action — the "clean non-verification component" the
  primary outcome was defined as. A turn that emits tool calls is the *act*
  (consulting `peer_b`, verifying its answer) and is excluded.
- **Continuation turns (`step == u32::MAX`) are included** per mika#1889 R4:
  they are text-only summarization turns, planning-class by construction, and
  `tool_use_in_turn` is always `false` for them, so the rule above already
  includes them. This is asserted by a test rather than by a special case.

Error and timeout turns carry zero tokens by construction, so they contribute
nothing and need no exclusion rule — declining to add one keeps the definition
free of a researcher degree of freedom.

**KTD3 — Confidence from the log, reliability from the manifest.** `--agent` is
an identity selector (mika#1888), so the confidence arm is encoded in `agent_id`:
`mika-dev-confidence-high` → 0.95, `mika-dev-confidence-low` → 0.55. Any other
`agent_id` is not an RT-005 run and its events are dropped. Reliability has no
log footprint and comes from the manifest. Reading each factor from its own
authority means a manifest that disagrees with the log cannot silently
mis-assign a run — the failure mode of carrying both in the manifest.

**KTD4 — Structural covariate separation via a sealed newtype (R2).** This is the
load-bearing correctness decision.

`PlanningTokens` is a tuple struct with a **private** field, defined inside a
private `estimand` submodule. Its only constructor is
`PlanningTokens::from_turns(&[TurnUsage])`, which applies KTD2. It implements no
`From<u64>`, no `From<Covariates>`, no arithmetic with anything but another
`PlanningTokens`. `Covariates` is a separate struct of `u32` counters that
exposes no method returning `PlanningTokens`.

The interaction function's signature accepts only `CellMeans`, which stores only
`PlanningTokens`. Therefore *there is no expression a contributor can write that
puts a covariate into the estimand* — feeding `handshakes` into the interaction
requires editing `from_turns`, which is a visible, reviewable act rather than an
accident. The report renderer takes the estimand and the covariate summary as two
distinct parameters and emits them under two distinct headings.

Rejected: a comment, a naming convention, or a runtime assertion. All three
permit the mistake and merely complain about it afterwards; R2 asks that the
mistake not compile.

**KTD5 — Verdict is a three-valued enum, magnitude is a separately named
accessor (R3).** The public result of the analysis is
`Verdict::{SignFlip, SameDirection, Degenerate}` plus the sign of each simple
effect. The raw contrast is reachable, because a reader must be able to
reproduce the arithmetic, but through an accessor named and documented as a
reproducibility diagnostic — never as an effect size. The rendered report leads
with the disclaimer, states the verdict, and prints the raw numbers under an
explicitly labelled diagnostics line.

**KTD6 — Covariate definitions are documented as unregistered proxies.**
`turns` = event count; `handshakes` = turns with `tool_use_in_turn == true`;
`recalculations` = turns with `tool_use_in_turn == false` that directly follow a
turn with `tool_use_in_turn == true` (re-deliberation after a tool result).
These are descriptive proxies chosen for mechanical computability, explicitly
**not** pre-registered, and carrying no authority over the estimand. Saying so in
the module docs is part of the deliverable: an undocumented covariate definition
invites later promotion to outcome.

### High-Level Technical Design

```
                       turn_usage JSON lines            run manifest
                                |                     (session -> reliability)
                                v                             |
                   parse_turn_usage_lines()  <-----------------+
                                |
                    group by session_id, drop unknown sessions
                                |
              +-----------------+------------------+
              |                                    |
   estimand::PlanningTokens::from_turns    Covariates::from_turns
   (output_tokens where !tool_use)         (turns, handshakes, recalcs)
              |                                    |
        CellMeans (2x2)                     CovariateSummary (2x2)
              |                                    |
        interaction() -> Interaction               |
              |                                    |
              +----------> render_report() <-------+
                                |
                    disclaimer (first) / verdict / covariates
```

Module layout in one file, `crates/mika-agent/src/research/mechanism_analyzer.rs`:

- `Confidence`, `Reliability`, `Cell` — the 2x2 coordinates.
- `TurnUsage` — the parsed subset of one log event.
- `parse_turn_usage_lines(&str) -> Vec<TurnUsage>` — R4/KTD1.
- `mod estimand` — the sealed `PlanningTokens` and `CellMeans` (KTD4).
- `Covariates`, `CovariateSummary` — KTD6.
- `Interaction`, `Verdict` — KTD5.
- `analyze(log: &str, manifest: &HashMap<String, Reliability>) -> Report`.
- `Report::render() -> String` — disclaimer first.

### Assumptions

- Brick 3/5 will emit a manifest mapping each run's `session_id` to its
  reliability arm. This plan fixes that contract; if brick 3/5 later records
  something structurally different, the analyzer's entry point adapts — the
  estimand does not.
- One RT-005 run corresponds to one `session_id`. This is how the agent loop
  labels a conversation and is what brick 3/5 will key on.
- The batch is captured with `MIKA_LOG_FORMAT=json` (the default for the
  server/gateway binaries). Under `pretty` the events are still emitted but not
  JSON — noted in the module docs as a capture prerequisite, not handled in code.

### Risks & Dependencies

- **The pilot may never run.** `operator-gated` on mika#1891 governs the *run*,
  not the build. The analyzer is built and tested against synthetic fixtures at
  the mika#1889 format; that is the whole verification surface here, by design.
- **Definition disputes.** KTD2 is a judgment call about where planning ends.
  Mitigation: it is stated once, in the module docs, with its rationale, and
  changing it means editing one function whose tests will fail loudly.
- **Covariate promotion pressure.** Once numbers exist, "handshakes went up" is a
  tempting headline. Mitigation is KTD4 plus the report's section split.

## Implementation Units

### U1. `research::mechanism_analyzer` — parser, sealed estimand, covariates, report

**Goal.** One module that turns a `turn_usage` log plus a run manifest into a
report whose primary claim is the sign behaviour of the confidence x reliability
interaction on planning tokens, and whose covariates cannot reach that claim.

**Requirements.** R1-R8.

**Files.**
- `crates/mika-agent/src/research/mechanism_analyzer.rs` (new).
- `crates/mika-agent/src/research/mod.rs` (add `pub mod mechanism_analyzer;`).

**Approach.**
1. Module docs stating: offline-only (R7), the pre-registered planning-token
   definition verbatim (KTD2) with its rationale, the covariate proxy definitions
   and their lack of authority (KTD6), the manifest contract (KTD3), and the
   existence-not-magnitude posture (R3).
2. `TurnUsage` via `serde::Deserialize` with `#[serde(default)]` on the numeric
   fields, parsed per line with `serde_json::from_str::<serde_json::Value>` gate
   on `event == "turn_usage"`; non-JSON and non-matching lines skipped.
3. `Confidence::from_agent_id` — the two RT-005 agent ids, `None` otherwise.
4. Private `mod estimand`: `PlanningTokens(u64)` with a private field and the
   single `from_turns` constructor applying KTD2; `CellMeans` holding four
   `Option<f64>` cell means derived only from `PlanningTokens`.
5. `Covariates::from_turns` and `CovariateSummary` — separate types, no bridge.
6. `interaction(&CellMeans) -> Interaction` computing the two simple effects and
   the contrast; `Verdict` from the two signs, `Degenerate` when any cell is
   empty.
7. `analyze` + `Report::render` — disclaimer block first, primary section, then
   a clearly separated covariate section.

**Test Scenarios.**
- Parses a verbatim flattened `turn_usage` line (timestamp/level/target present);
  ignores a `system_prompt_assembled` line and a non-JSON line in the same input.
- A continuation turn (`step = 4294967295`) contributes to planning tokens (R6).
- A turn with `tool_use_in_turn = true` contributes zero planning tokens and
  exactly one handshake.
- `input_tokens` does not affect planning tokens: two fixtures identical except
  for `input_tokens` yield the same estimand (pins KTD2 and the provider-asymmetry
  immunity).
- Opposite-signed simple effects → `Verdict::SignFlip`; same-signed →
  `SameDirection`; an empty cell → `Degenerate` with no panic.
- A `session_id` absent from the manifest is dropped, not assigned.
- An unrecognized `agent_id` is dropped.
- `render()`'s first non-empty lines are the disclaimer, and the primary-estimand
  section contains no covariate name.

**Verification.** `cargo test -p mika-agent research::mechanism_analyzer`,
`cargo clippy -p mika-agent --all-targets -- -D warnings`, `cargo fmt --check`.

## Verification Contract

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test -p mika-agent research::` — new tests pass, `peer_b`'s existing
  tests unaffected.
- `git diff --stat` against `main` touches only:
  `crates/mika-agent/src/research/mechanism_analyzer.rs`,
  `crates/mika-agent/src/research/mod.rs`, `docs/plans/...`, `docs/solutions/...`.
- `bash scripts/verify-pipeline.sh` — passes (docs + source buckets both present).
- **Guardrail 2 is verified by the compiler**, not only by a test: the reviewer
  can confirm no public constructor of `PlanningTokens` accepts an arbitrary
  number.

## Definition of Done

- The module compiles, is declared in `research/mod.rs`, and is reachable from
  nothing else — no agent-loop wiring, no tool registration, no CLI surface.
- The pre-registered planning-token definition appears once in the module docs
  and once in code, and they agree.
- The report renders the external-validity disclaimer before any number.
- Covariates appear only under their own heading and cannot be typed into the
  estimand.
- No abandoned or experimental analysis path is left in the diff — one
  definition, one estimand, one verdict.
- No run of the RT-005 protocol was executed to produce any of it.

## Acceptance criteria

- [ ] Parse les logs tokens par tour (format de 4/5).
- [ ] Calcule le terme d'interaction confiance×fiabilité sur les tokens-planification.
- [ ] Rapporte les covariables séparément, sans les mélanger à l'estimand.
- [ ] Output = preuve d'existence (signe/bascule) + disclaimer magnitude.
