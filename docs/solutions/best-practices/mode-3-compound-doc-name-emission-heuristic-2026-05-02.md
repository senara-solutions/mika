---
title: "Mode 3 emission heuristic — compound-doc-name labels are not findings; do NOT iterate plans against them"
date: 2026-05-02
category: best-practices
problem_type: workflow_issue
module: mika-arch
component: skills
tags:
  - mika-arch
  - mode-3
  - contract-fabrication
  - ratification-trap
  - peer-review
  - operator-discipline
  - architect-behavior
applies_when:
  - Reading a mika-arch / mika-qa / reviewer response to a grooming or review brief
  - Findings appear as snake_case labels without bodies
  - Labels engage with the brief's open questions (look "substantive" because they touch the content)
  - Tempted to infer finding bodies from the labels and iterate the plan
---

# Mode 3 emission heuristic — compound-doc-name labels are not findings; do NOT iterate plans against them

## Context

mika#927 grooming brief (Stage-2 fairness budget, factor-of-2 oversupply, btreemap log fields, ratio fairness assertions) hit mika-arch first pass at session `95e5e97c-1583-4694-9949-b6f9bfe7ea93` on 2026-05-02. The response was 761 chars and returned four "findings" as snake_case labels:

- `round_robin_interleave_algorithm_kimi_fabrication_risk`
- `factor_of_2_oversupply_stage2_budget_miss_heavy_corpus_defense`
- `btreemap_before_serialize_for_deterministic_log_fields`
- `ratio_assertion_shape_for_fairness_invariant_tests`

Each label engaged with a real open question from the brief. None had a finding body. The temptation: *"the model touched the content, the labels are inferred review findings, iterate the plan against them."*

Peer review (`mika-ask-a-friend` brief, today's session `cf208770`) rejected the iteration: *"'engaged with the content' isn't proof of substantive review; it's proof the model touched the content while producing the wrong shape. Don't update on it."* Classified as Mode 3 contract-fabrication, instance 2 in `~/.claude/projects/-data-workspace-mika-platform/memory/project_mika_arch_failure_modes.md`'s Mode 3 entry.

The shape is recognizable: snake_case `<topic>_<context>_<action>`, broadly-applicable framing, each label readable as a proposed compound-doc filename. The model has emitted *the meta-commentary about what to compound from a review* in place of the review itself.

This is the third manifestation of the failure family catalogued in `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` ("availability hallucination", "sufficiency hallucination", and now "compound-doc-name fabrication"). That doc's Rule 3 — *"the catalogue is necessary but not sufficient"* — is what makes Mode 3 detection load-bearing: prompt-level catalogues of past failures don't bind future behavior under load even when the agent has just read them. The structural counter (mika#864 required-suffix-line guard) was satisfied — the response did emit `Disposition: ITERATE` — but the content the counter wraps was fabricated.

## Guidance

When an architect or reviewer response returns findings as snake_case labels without bodies — especially with the broadly-applicable `<topic>_<context>_<action>` shape — classify as Mode 3 contract-fabrication. Do **not** iterate the plan against the labels.

| Step | Action |
|---|---|
| 1. Recognize the shape | Labels are snake_case, 4-7 tokens, read as compound-doc filenames. Each touches a real brief topic. Bodies are absent. |
| 2. Do not infer bodies | Operator-inferred bodies attributed to the architect = operator authoring review findings under architect's name (the ratification trap from `feedback_peer_review_ratification_vs_discovery`). |
| 3. Classify and surface | Mark as Mode 3, instance N+1 in `project_mika_arch_failure_modes.md`. Surface the session ID to the skill maintainer (Vincent / mika-arch skill author at mika#901). |
| 4. Do not retry pass-1 | Treat as a contract violation that produced no review at all. The disposition is "halt + bounded-B at the skill-defect timeline" — see sibling compound `bounded-b-fallback-operator-cadence-enforced-2026-05-02.md`. |
| 5. Do not reset the pass count | See sibling compound `f8c-sibling-loophole-pass-quality-vs-pass-count-2026-05-02.md`. |

The recognition test in one line: *"if I copied this label into `docs/solutions/best-practices/<label>-2026-MM-DD.md`, would it look like a normal compound-doc filename?"* If yes, the model emitted a filename, not a finding. (Heuristic preserved verbatim from `project_mika_arch_failure_modes.md` Mode 3 anti-pattern entry.)

## Why This Matters

Mode 3 contract-fabrication is the failure mode where the architect skill produces output that looks structurally like the contract (review findings) but is actually a sibling artifact (compound-doc-name shorthand, meta-commentary, restated brief). It's harder to detect than thin-emission Mode 1 or off-topic Mode 2 because the labels engage with the brief's actual content — that's what makes operators want to update on them.

The ratification trap (auto-memory `feedback_peer_review_ratification_vs_discovery`) is when an operator restates their own pre-review uncertainty as the reviewer's discovery. Mode 3 with operator-inferred bodies is the **same trap one layer down**: the operator infers what the reviewer must have meant by `factor_of_2_oversupply_stage2_budget_miss_heavy_corpus_defense`, writes that body, attributes it to mika-arch, and iterates the plan. The plan now has a "review finding" the architect never authored. Future readers can't tell the finding's provenance. The audit trail is corrupted at the source.

The cost compounds. Every Mode 3 instance the operator iterates against teaches the platform that "label-as-finding" is acceptable input. The next architect skill version trained / prompt-tuned on these traces will produce more labels, fewer bodies. The contract erodes from both sides — model emits labels because labels worked; operator accepts labels because the model emitted them.

This is why mika#864's structural counter (required-suffix-line guard) is necessary but not sufficient — it binds emission **shape** but says nothing about emission **content provenance**. mika#901's pending fix (emit findings verbatim, not only persist to memory) closes the next axis. Even after #901 ships, "emit verbatim" doesn't prevent the model from emitting *fabricated verbatim findings* (claiming "F1: ..." that wasn't actually derived this turn). #901 is necessary but not sufficient too. The third axis — provenance/causation — is what `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` (memory-shared isolation axis) and `mika/docs/solutions/best-practices/carve-out-trigger-outcome-shape-vs-causation-2026-04-30.md` formalize. Mode 3 lives at the intersection of these three.

The recognition heuristic short-circuits the trap. Once the shape is identifiable in one glance — snake_case, broadly-applicable, no body — the operator classifies and halts before any inference happens.

## When to Apply

- **First-pass mika-arch response on a grooming brief.** Read the response. If findings are snake_case labels without bodies, apply the heuristic.
- **First-pass mika-qa or any reviewer response with a "findings" contract.** Same shape detection works.
- **Second-pass response after a retry.** Especially watch here — Mode 3 in pass-1 followed by Mode 3 in pass-2 is a stronger skill-defect signal, not a "model is converging" signal.
- **Reviewing a plan doc that cites architect findings.** If the cited findings read as snake_case label-shaped phrases rather than full sentences, check the architect transcript — the operator may have inferred bodies the architect didn't write.
- **NOT applicable** when the architect emits real prose findings that *also* include a recommended doc filename. The combo is fine — the doc-filename suggestion is supplementary, not the finding itself.
- **NOT applicable** when the labels appear in a non-contract context (e.g., a `store_fact` row, a debug trace, a meta-commentary section explicitly framed as such). The trap is when labels appear *in place of* findings under a "Findings:" header.

## Examples

### Today's incident (mika#927, session `95e5e97c-1583-4694-9949-b6f9bfe7ea93`)

Architect response (761 chars total, paraphrased structure):
```
Findings:
1. round_robin_interleave_algorithm_kimi_fabrication_risk
2. factor_of_2_oversupply_stage2_budget_miss_heavy_corpus_defense
3. btreemap_before_serialize_for_deterministic_log_fields
4. ratio_assertion_shape_for_fairness_invariant_tests

Disposition: ITERATE
```

Each label maps to a real open question from the brief. None has a body. Mid-thread temptation: *"okay, finding 1 means kimi might fabricate the round-robin algorithm, so we should add an algorithm-stability check to the plan; finding 2 means the factor-of-2 budget might miss heavy-corpus defense, so we should add a heavy-corpus test case…"*

Peer review response: *"'engaged with the content' isn't proof of substantive review; it's proof the model touched the content while producing the wrong shape. Don't update on it."*

Correct response: classify Mode 3 instance 2, surface to skill maintainer, halt with bounded-B at skill-defect timeline.

### Prior instance (mika#918, session `1918cb84-c3fc-4514-b0cb-e55ef4b99b19`, 2026-05-01)

Earlier session on a different ticket produced the same shape: labels matching brief topics, no bodies, suffix-line satisfied. Logged as Mode 3 instance 1 in `project_mika_arch_failure_modes.md`.

### Counter-example (correct architect output for comparison)

A real architect finding has a body. Same `factor_of_2_oversupply` topic, real shape:

> *"Stage-2 budget at factor-of-2 oversupply: the brief assumes a uniform corpus, but heavy-corpus agents (mika-arch, mika-qa with KG-v27 enabled) breach the budget at ~1.7x. Recommend: add a heavy-corpus stress case to the budget defense; the assertion should be `actual_oversupply / theoretical_oversupply <= 2.1` to preserve the factor-of-2 guarantee under heavy load."*

This has a body. It cites the brief's assumption, names the breach point, and proposes a concrete plan amendment. Iterating against this is correct; iterating against the label `factor_of_2_oversupply_stage2_budget_miss_heavy_corpus_defense` is the trap.

### Recognition shortcut

```
# If the response's findings section, copied verbatim, would look like:
#   docs/solutions/best-practices/<label>-2026-05-02.md
# … the labels are filenames, not findings. Mode 3.
```

## Reference

- **Today's incident:** mika#927 first-pass architect session `95e5e97c-1583-4694-9949-b6f9bfe7ea93` (2026-05-02), 761 chars.
- **Prior instance:** mika#918 first-pass architect session `1918cb84-c3fc-4514-b0cb-e55ef4b99b19` (2026-05-01).
- **Peer-review precedent** (today's `cf208770` line ~330): *"'engaged with the content' isn't proof of substantive review; it's proof the model touched the content while producing the wrong shape. Don't update on it."*
- **Umbrella catalogue** `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Mode 3 is the predicted third manifestation; this doc is the next entry in that catalogue.
- **Immediate predecessor** `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` — thin-emission recovery via `tool_calls.store_fact` rows. Mode 3 is its evolution from thin to fabricated-prior-emission.
- **Memory-shared coupling** `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — explains how Mode 3 arises (architect treating its own persisted reasoning as prior emission even when no surface emission occurred).
- **Provenance discriminator** `mika/docs/solutions/best-practices/carve-out-trigger-outcome-shape-vs-causation-2026-04-30.md` — outcome-shape vs causation; Mode 3 satisfies outcome-shape (Disposition line present) but fails on causation (no this-turn derivation).
- **Structural floor** `mika/docs/solutions/best-practices/required-suffix-line-guard-verdict-ghosting-structural-fix-2026-04-29.md` — mika#864 structural counter that Mode 3 satisfies in shape but violates in content; Mode 3 sharpens that doc's "model cannot rationalize past a structural check" claim.
- **Pre-commit ratification baseline** `mika-platform/docs/solutions/dev-loop/2026-04-27-pre-commit-peer-review-converts-iteration-cycles.md` — the success pattern Mode 3 perverts ("ratification" → "fabricated ratification").
- **Sibling compound** `mika-platform/docs/solutions/best-practices/bounded-b-fallback-operator-cadence-enforced-2026-05-02.md` (this batch) — disposition of a Mode 3 halt; bounded-B at skill-defect timeline.
- **Sibling compound** `mika-platform/docs/solutions/best-practices/f8c-sibling-loophole-pass-quality-vs-pass-count-2026-05-02.md` (this batch) — why "this Mode 3 doesn't burn a pass" is the same loophole shape; do NOT reset.
- **Auto-memory companion** `feedback_peer_review_ratification_vs_discovery` — the ratification trap one layer up; this lesson is its mode-3-specific instance.
- **Auto-memory companion** `project_mika_arch_failure_modes.md` — Mode 3 retry-policy entry (inline-fragment); this doc is the durable citation handle for the recognition heuristic.
- **Open tickets** mika#901 (emit findings verbatim — pending structural fix), mika#864 (suffix-line guard — already shipped), mika-platform#70 (axis-count instrumentation — investigates correlation).

### Future surgical edit prediction

After mika#901 ships (verbatim emission guard), if Mode 3 recurs as fabricated-verbatim findings rather than label-shorthand, the recognition heuristic above no longer fires (labels become fake bodies). The next axis is **causation/provenance** — verifying findings reference content emitted *this turn* via cross-check against `tool_calls.store_fact` rows or message body. Per the architect's `current_priorities` core memory, that's a separate structural enhancement after #901, likely mika#9XX. This compound stays load-bearing in the meantime; revisit when #901 ships.
