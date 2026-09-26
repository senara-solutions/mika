---
title: "Sealing a pre-registered estimand: one constructor, one unavoidable pair, and a field with no asymmetry"
module: mika-agent::research
tags: [measurement, pre-registration, newtype, structural-guard, token-accounting, reporting-rule, rt-005]
problem_type: bug-class-prevention
category: best-practices
date: 2026-08-29
issue: 1891
---

# Sealing a pre-registered estimand

Three lessons from building the RT-005 offline analyzer (mika#1891), all about
protecting a *measurement definition* rather than a security boundary. The
common shape: the protocol document already said the right thing, and the code
was changed so the wrong thing stopped being expressible.

## 1. A pre-registered definition needs one constructor, not a convention

RT-005 pre-registered exactly one primary outcome — planning tokens — and three
descriptive covariates (turns, handshakes, recalculations) that must never enter
it. The tempting implementation is four counters in one struct plus a rule, in a
comment, saying which one is the outcome.

That rule survives exactly as long as nobody is in a hurry. Any expression that
adds `handshakes` to the primary metric compiles, and the result is not a crash —
it is a plausible-looking number that silently answers a different question than
the one that was pre-registered.

The fix is to make the wrong expression fail to compile:

```rust
mod estimand {
    /// Constructible ONLY via `from_turns`, which is where the definition lives.
    pub struct PlanningTokens(u64);   // private field, private module

    impl PlanningTokens {
        pub fn from_turns(turns: &[&TurnUsage]) -> Self { /* THE definition */ }
    }

    impl CellMean {
        pub fn of(runs: &[PlanningTokens]) -> Self { … }   // accepts nothing else
    }
}
```

No `From<u64>`, no `From<Covariates>`, no arithmetic across the two types. The
covariate struct exposes no method returning `PlanningTokens`. To move a
covariate into the estimand you must edit `from_turns` — a visible, reviewable
act rather than an accident, which is the whole property that was wanted.

This is the same mechanism as
[structural-guards-vs-doc-comments](structural-guards-vs-doc-comments-2026-06-13.md)
(mika#755's `AgentScopedTaskId`), applied to a different kind of invariant: that
one protects *who may act on a resource*, this one protects *what a number
means*. Worth naming separately because the failure mode differs — an ownership
violation surfaces as a wrong answer to a right question, a definition violation
surfaces as a right-looking answer to a question nobody asked.

**Generalizes to:** any metric that is fixed before the data exists — an A/B
primary outcome, an SLO's numerator, a billing quantity. If the definition can be
reached by more than one code path, there is more than one definition.

## 2. Prefer the field that has no asymmetry over normalizing one that does

`docs/solutions/best-practices/cross-provider-input-tokens-cache-inclusion-asymmetry-2026-08-20.md`
(mika#1889) documents that `LlmUsage.input_tokens` means different things per
provider family: Anthropic reports fresh input, OpenAI-compat reports
`prompt_tokens`, which already includes `cache_read`. Its prescription is that
every consumer applies a per-family correction keyed on `provider`.

The analyzer needs none of that, because its metric sums `output_tokens` only.
The asymmetry is *entirely* about what `input_tokens` includes, so a metric that
never reads that field cannot be miscounted by it. The correction is not applied
carefully — it is not needed at all.

Choosing `output_tokens` was independently the right call on the merits (output
tokens are what the agent produced; input tokens are context handed to it and
mostly measure prompt size). But it is worth noticing when a substantive choice
*also* removes a known correctness hazard, and worth pinning so the property
does not quietly lapse:

```rust
#[test]
fn input_tokens_never_reach_the_estimand() {
    let inflated = base.replace(r#""input_tokens":9999"#, r#""input_tokens":999999"#);
    assert_eq!(analyze(&[run(base)])…, analyze(&[run(inflated)])…);
}
```

**The rule:** when a data source has a documented per-source semantic
disagreement, check first whether the metric can be defined off a field that
does not carry it. A normalization you don't have to write is one that cannot
drift, and the compensating code is where the drift lives — someone extends the
metric, forgets the correction, and the miscount returns silently.

## 3. When the protocol says "report both", make one-alone unreachable

RT-005's pre-registration names two contrasts on the same metric — the labelled
arm (intention-to-treat) and the realised perturbation — and adds a rule:

> Reporting only one is a protocol violation, whichever one it is.

The rule exists because the design is balanced on the *label*, not on the
*manipulation*: `peer_b` perturbs 3 of 10 items, so 14 of the 20 runs in a
degraded cell are byte-identical in input to their fiable counterparts. The
primary contrast is diluted by construction and the secondary is not. Whoever
reports one alone gets to pick the more favourable answer, which is the
garden-fork the pre-registration was written to close.

Naming both in a document closes it for an honest reader. Closing it in the code
means the API cannot express the violation:

```rust
impl Report {
    /// Both verdicts, as a pair: primary first, pre-specified secondary second.
    pub fn verdicts(&self) -> (Verdict, Verdict) { … }   // the ONLY accessor
    pub fn render(&self) -> String { /* emits both, primary first */ }
}
```

No `primary()`. No `secondary()`. `render()` has no flag selecting one. A test
exists whose only job is to fail if someone adds a single-contrast accessor.

**The distinction that keeps this legal.** A guardrail on this pilot forbids "a
family of tests" — and two contrasts could look like the start of one. They are
not, on two counts: both were named before any data existed, and they share one
metric and one interaction form, differing only in which runs enter. What makes
a family dangerous is *choosing among its members after seeing the results*, and
that choice is exactly what has been removed. Pre-registering two and emitting
both is the opposite of a garden fork; pre-registering one and quietly computing
a second is the fork.

**Generalizes to:** any reporting duty phrased as "always report X alongside Y" —
a fairness metric beside an accuracy metric, a p-value beside an effect size, a
cost beside a latency. If the code can hand back one, someone eventually will,
and the omission will not look like an omission.

## Where this lives

`crates/mika-agent/src/research/mechanism_analyzer.rs` (mika#1891), alongside
`peer_b.rs` (mika#1887). Both are disposable experiment apparatus under the
`research` module doctrine — the patterns above are the keepers, not the code.
