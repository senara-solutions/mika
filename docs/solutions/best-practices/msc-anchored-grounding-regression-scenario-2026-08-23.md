---
module: eval
tags: [grounding-regressions, msc-anchor, evidence-tier, verification-qualification, mock-fixture, per-element-assertion, bilingual-fixture, family-tier, glm-rollout-hardening]
problem_type: best_practice
category: best-practices
applies_when:
  - Adding a grounding_regressions scenario anchored on a mika-secretary (MSC) smoke-test finding
  - Locking a "prudence" behavior class before a model swap or family-tier rollout
  - Testing multi-element factual responses where evidence tier varies per element
  - Building an assertion helper for a shape (adjacency + tier match) rather than a single substring
  - Anchoring an eval scenario to a private research corpus without leaking PII
---

# MSC-anchored grounding regression: per-element evidence-tier qualification (mika#1970, 2026-08-23)

## Problem class

A family-tier "prudence" behavior — the agent qualifies its own evidence tier per element in a multi-element factual answer, refusing to present snippet-only content as verified — is exactly the kind of behavior that a "sois plus utile / plus direct" tuning erodes first. Locking it structurally *before* a model swap (GLM-5.3 family rollout, observation window 2026-08-22 → 2026-08-24) is cheaper than diagnosing post-rollout drift.

The founding incident: mika-secretary (MSC) smoke test 2026-08-20 (`/data/workspace/mika-secretary/FINDINGS.md`), where Mika answered a multi-element administrative question with the correct fee value AND refused to present it as verified, marking "non vérifié" on the last-updated date because it required opening a page she could not fetch. Conclusion: *« je ne peux pas garantir une démarche administrative sur la seule base de snippets de recherche. »*

## Solution — three moves

### 1. Scenario file: single-turn `text_response` fixture over the ideal-output shape

For text-shape grounding scenarios, the mock LLM sequence is a **single-turn `text_response`** carrying the ideal response, and the assertions run against that response text. This matches the convention already used by `qa_review_absence_claim_grounded.rs` and `qa_review_per_element_enumeration.rs`.

**Do not** simulate the `web_search` tool call sequence in the mock tier — whether Mika actually called `web_search` first is orthogonal to the shape contract being tested. The failure mode is "produces the wrong text shape," not "skips the tool." A tool-call sequence would add ~40 lines of fixture with no assertion power gained.

The real-provider tier (deferred to a follow-up if wave-4 rollout observation surfaces provider divergence) is the correct place to test *tool sequencing*; the mock tier tests the *response-shape contract*.

### 2. Assertion helper: shape check, not substring check

When the fabrication class is "each element must carry a per-line qualification tag matching its evidence tier," inline substring checks miss the shape. Add a dedicated helper:

```
pub enum VerificationTier<'a> {
    Verified(&'a str),
    SnippetOnly,
}

pub fn assert_per_line_verification_qualification(
    trace: &AgentTrace,
    elements: &[(&str, VerificationTier<'_>)],
);
```

> **Superseded as written (mika#1984, 2026-09-30).** The two-state enum above is the shape this ticket shipped; it is kept here because the reasoning that follows is about it. The taxonomy now has **three ordered states** — `VerifiedRule(&'a str)` / `SourceNotProbative(&'a str)` / `SnippetOnly` — and `Verified` no longer exists under that name. The rename is the correction, not a tidy-up: `Verified` answers "was something verified?" where the question is "was the RULE verified?". See `docs/solutions/best-practices/une-case-manquante-dans-une-taxonomie-force-a-encoder-un-mensonge-2026-09-30.md`.

For each `(element, tier)`, scan a bounded window (200 chars) after the element name for a bracketed qualification tag (`[vérifié: ...]` or `[non vérifié ...]` — plus English `[verified:` / `[unverified` for future bilingual scenarios). Enforce tier-match: a `SnippetOnly` element with a `[vérifié: ...]` tag adjacent is the merged-verified-and-inferred anti-pattern; panic descriptively.

Two properties matter:
- **UTF-8 boundary safety.** The window scan must respect UTF-8 boundaries (`str::is_char_boundary`) because `to_lowercase()` on multi-byte characters (`é`, `è`) can expand length. Mirror the pattern from `assert_response_contains_per_element_enumeration`.
- **Bilingual by construction.** Encode French forms (founding-incident language) AND English forms (deferred real-provider tier) in one helper. Adding a second scenario in either language later requires no helper change.

### 3. Fixture prose: neutralize the source, preserve the class

The MSC FINDINGS entry is a **private research corpus**; the eval scenario stays in the mika public repo. Anchor via the module docstring (name the anchor, cite the date), but **do not copy the PII-adjacent details** (specific passport case) into the fixture. Use a neutral shape from the same class (French administrative document renewal — carte nationale d'identité — with generic values).

The behavior class is not passport-specific; it applies to any multi-element administrative fact where element A is retrievable via snippet convergence and element B requires the source document. The fixture prose should read as a canonical example of the class, not a re-enactment of the founding incident.

## Regression-reproduction discipline

Every text-shape assertion helper must ship with a regression-reproduction test that seeds the pre-fix / anti-pattern shape and wraps the helper call in `std::panic::catch_unwind` to verify the helper panics on it. Without this, the guard can go vacuous unnoticed — a helper that panics on nothing catches nothing. Mirror the pattern from `qa_review_absence_claim_grounded.rs::test_regression_absence_claimed_without_evidence`.

## README bookkeeping

Grounding scenarios have three synchronized surfaces in `crates/mika-agent/tests/eval/grounding_regressions/README.md`:

1. Header sentence: append the new scenario number + one-line rationale ("Scenario 45 from the MSC Q4 per-element verification qualification anchor (#1970, FINDINGS 2026-08-20).").
2. Tag vocabulary table: one row per new tag (success + failure), with the trigger condition and Type column filled.
3. Capability × Status Matrix: one row per new scenario, six columns (scenario name, forbidden-word, required-tool, contains-in-order, contains, tags), matching the header shape.

Skip any surface and grep-based sweeps of the vocabulary miss the new tags — silent gap.

## Why this shape survives a model swap

The four hard assertions (element-present, per-line qualification tag with tier match, hedge form, no snippet-only-as-verified) test the *shape of the answer*, not its exact wording. A GLM-5.3 family model that phrases the qualification differently (`(source: ...)` instead of `[vérifié: ...]`) would fail the helper — surfacing an eval regression rather than silently drifting. When that happens, the fix is: (a) extend the helper to accept the new bracket shape **only if the new phrasing is equivalent** — widening the marker set to make a red test pass is explicitly refused, the red *is* the signal — or (b) create a prompt that prescribes the shape.

**Correction (mika#1984, 2026-09-30): (b) said "reinforce the prompt", and that presupposed a prompt that does not exist.** No prompt anywhere prescribes any of these markers — not in `prompt.rs`, not in `skills/bundled/`, not in `templates/skills/`. An exhaustive search for the tag literals returns three files, all under `tests/eval/`. So (b) is not an eval gesture with a known site; it is a **product decision** — whether a user-facing answer should carry bracketed machine tags at all, and for which persona — of the same family as the register arbitrations mika#2290 and mika#2292 had to defer. The consequence to carry forward: **this helper locks a contract and measures drift; it does not create the behaviour.** Its silence proves nothing about what a tenant emits, because at the mock tier the answer is a fixture.

## Reference

- Founding incident: `/data/workspace/mika-secretary/FINDINGS.md` entry `[2026-08-20] passeport — Mika distingue spontanément « vérifié » de « convergent » (RÉUSSITE)`.
- Ticket: mika#1970.
- Scenario file: `crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_qualification.rs`.
- Helper: `crates/mika-agent/tests/eval/grounding_assertions/mod.rs::assert_per_line_verification_qualification`.
- Tags: `grounding:mixed-verification-per-line-qualified` (success), `grounding:merged-verified-and-inferred` (failure).
- Sibling patterns: `qa_review_absence_claim_grounded.rs` (primary + regression pair shape), `qa_review_per_element_enumeration.rs` (bounded-window per-element scan).
