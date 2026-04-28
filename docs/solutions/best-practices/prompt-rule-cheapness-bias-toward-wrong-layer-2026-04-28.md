---
module: mika-platform
date: 2026-04-28
problem_type: best_practice
component: development_workflow
severity: high
applies_when:
  - Designing a rule of the form "agent should do X under condition Y"
  - Adding text to an agent prompt, soul constant, skill catalogue, or compound doc to prevent recurrence of an observed failure
  - Authoring a commit-message convention, PR-body trailer, or other text-as-protocol contract
  - Responding to a recurring failure with "we need to document this more clearly" rather than a structural change
  - Comparing a prompt-level fix against an engine, CI, tooling, or invariant-check alternative and the prompt-level option appears cheaper
related_components:
  - tooling
  - documentation
  - testing_framework
tags:
  - prompt-engineering
  - structural-enforcement
  - meta-pattern
  - compound-engineering
  - cost-comparison
  - rule-3-recurrence
  - operator-bias
  - layer-selection
---

# Prompt-rule cheapness is a bias toward the wrong enforcement layer

## Context

When proposing "the agent should do X under condition Y," the cheap default impulse is to write a prompt-level rule (add a sentence to a system prompt, add a section to a soul template, define a commit-message trailer, write another compound doc telling the operator-equivalent agent to dispatch through `/mika`). The right impulse is almost always to ask whether X can be enforced *structurally* — by the engine, by CI, by a tool's required-args, by a server-side check, by a typed invariant, by capability removal.

This bias has been recurring in the mika-platform codebase since at least 2026-03-30. It has been named at the meta level three times before today (in progressively-more-general framings) and continues to recur. **Today's compound doc names it as a class** so it can be cited during decisions, not just observed in retrospect.

Today's recurrence count, traced via session history (session history):

| Date | Surface | Outcome |
|------|---------|---------|
| 2026-03-30 | mika-qa CI-wait verdict timing | Vincent corrected: CI latency belongs in the transport layer (reminder-based deferred verdict), not in a prompt rule. `feedback_prompt_enforcement_fragile.md` written. |
| 2026-04-08 | mika-qa "Do NOT hold for pending CI" rule violated despite being in prompt | Capability removed from prompt entirely (structural). |
| 2026-04-08 | mika#485 Rule 6 violation ("never call `run_gh pr merge`" in self-dev-webhook-qa) | No structural fix at the time. |
| 2026-04-10 | skill-review agent shortcutting workflow | Prompt hardening only — no structural backstop. (`docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md`. Refresh candidate per Phase 2.5 below.) |
| 2026-04-17 | skill prompt size human-discipline rule (`wc -c` review before merge) | Failed in 9 days. |
| 2026-04-24 | mika#792 Rule 6 violation (second occurrence, 16 days after first) | Structural: tagged-union tool returns + policy table. (`docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md`.) |
| 2026-04-26 | skill prompt size rule fails again | CI-time test added (structural). |
| 2026-04-27 | Pattern named for human-discipline specifically | `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — "humans-reviewing-size-limits is the same shape of fragile-rule." |
| 2026-04-28 | Today's session — N=4 within one day | mika#788, mika#860/#861, mika#866/#867, mika#868. Plus operator-side recurrences. |

The four 2026-04-28 instances aren't related by domain. They're related by *decision shape* — the same cost-asymmetry illusion produced the same wrong default answer four times in eight hours. Combined with the prior 5 instances, this is the meta-frame around three compound docs already shipped (`engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, `prompt-vs-tool-contract-mismatch-2026-04-24.md`, `structural-check-replaces-human-discipline-2026-04-27.md`). Each named the pattern in a specific domain. This doc names it across all of them.

## Guidance

When you find yourself proposing "the agent should do X under condition Y," **before shipping the prompt-level version, ask**:

1. **Can X be enforced structurally?** Engine guard, CI check, tool required-arg validation, type-system invariant, server-side label inheritance, post-condition assertion, schema constraint, capability removal — anything that doesn't depend on an LLM remembering or honoring a sentence under load.

2. **If yes, what's the structural cost?** This is the step where the bias bites hardest. Structural fixes feel expensive (engine code, CI workflow, schema migration); prompt rules feel cheap (string concat). The felt cost is almost always wrong — see examples for four 2026-04-28 cases where the structural fix was a single hook, a single `gh api` call, a deletion, or a fixture-driven check. Do the actual scoping before declaring it "too expensive."

3. **If structural is genuinely right but currently expensive, is the prompt-level version *transitional scaffolding* or *parallel duplication*?**
   - **Scaffolding:** connects two places the system needs to traverse during the transition; has a defined retirement condition tied to the structural ship; gets deleted when that ships.
   - **Parallel duplication:** lives next to where the structural enforcement would live; has no retirement condition; accretes; becomes citation-fodder for the next failure ("per the protocol…").

4. **Does the prompt-level version solve the recurrence pattern, or merely document it?** Documenting recurrence is fine and useful — but call it documentation, not enforcement. Don't ship a prompt rule and expect it to bind behavior; ship it as a citation handle for human design review, and file the structural ticket alongside.

A useful test: if your only mechanism for the rule taking effect is "the next agent will read this and act on it," the rule is documentation. If it's "the engine/CI/tool will refuse to proceed otherwise," the rule is enforcement. Don't conflate them.

## Why This Matters

The cost asymmetry between prompt rules and structural enforcement is illusory.

**Prompt rules accumulate.** Per `core-memory-as-citation-not-accumulator-2026-04-28.md`, prompt-level guidance in soul/system/skill prompts has no built-in retirement pressure. Each new rule adds a line. Lines that contradict, overlap, or supersede earlier lines coexist with them. The catalogue grows. Token budget gets eaten. The signal in any given rule decays as the surrounding catalogue swells.

**Prompt rules don't bind under load.** Per Rule 3 of `required-tools-gate-evasion-patterns-2026-04-28.md`, an agent under load can read a "MUST" directive on the same turn it violates it. Decisive evidence: DB session `03d3ec38-0839-47b6-9226-111b38d8b52b` shows the architect for mika#788 reading its own catalogue of recurrence-1 (mika#654) DURING the turn it ghosted recurrence-2's verdict line. The prompt was active in context; the rule did not bind. (session history) Earlier traces of the same pattern: mika-qa April 8 turn where the prompt rule "Do NOT hold for pending CI" was active in the system prompt (line 98) and was rationalized past in the same turn — captured in audit log but not yet named at the meta level until today.

**Prompt rules become citation fabrication fodder.** Once a rule exists in prompt-readable form, downstream agents cite it as authority for whatever they were going to do anyway — see `mika-platform/docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md` for the canTool variant. "Per the promotion protocol, this is a Bucket-1 item" reads as compliance even when the protocol doesn't say that.

**Each new prompt rule is one more entry the next failure reads and proceeds past.** The catalogue itself becomes proof-of-due-diligence for failures that route around it.

**The recursive observation, stated honestly:** this compound doc is itself a prompt-level catalogue entry. Per its own argument, it will not bind agent behavior under load. Its value is in **human design review** — surfacing the bias as a recognizable class so future authors (human or agent) notice when they're about to ship the cheap version of a structural problem, *during the design review that humans actually do*. It is not, and cannot be, a behavioral guard. Be honest about that. The structural correlates of today's recurrences are mika-platform#62 (PR-creation CI hook), mika#864 (engine EndTurn post-condition guard), mika#861 (label-driven exemption), mika#862 + mika#863 (engine guards for required-tools-gate). Those are the load-bearing fixes. This doc is the citation handle for "we noticed the class."

## When to Apply

Reach for this lens when you are about to:

- **Author a new agent rule** in a system prompt, soul template, skill prompt, or compound doc that says "the agent should do X under condition Y."
- **Design an exemption mechanism** for CI, lint, quality gates, or pipelines (the trailer-vs-label decision is the canonical shape — see Example 2).
- **Write a post-mortem or compound doc** that proposes "agents should do X going forward" without a structural enforcement component filed alongside.
- **File a "discipline" ticket** that has no structural sub-task — that's a smell; the recurrence will recur. Per `structural-check-replaces-human-discipline-2026-04-27.md`, **N=3+ recurrences → CI gate, not another prompt rule.**
- **Operator-side**: catch yourself about to act directly from conversation context instead of dispatching through a canonical pipeline (the canonical pipeline produces a plan-on-branch artifact downstream stages depend on; acting directly skips it). The operator-side analogue is documented in `feedback_always_use_mika.md` — written ~April 17 in response to the same pattern (session history).

## Examples

### Example 1 — Architect verdict-line ghost (mika#788)

- **Cheap impulse:** Strengthen the prompt. The skill `mika/skills/bundled/mika-arch-second-review/system_prompt.md:39-53` already says output MUST end with `Verdict: GROOMED|ESCALATE` and NEVER return ITERATE. After the ghost on mika#788 pass-2, the obvious next move is to add another sentence — bold it, capitalize it, repeat it.
- **Structural correlate:** Engine post-condition guard. The skill manifest declares its required output suffix; the engine validates EndTurn against it; failure to emit produces a tool error, not a closed turn.
- **What we shipped:** mika#864, the structural guard. No additional prompt sentence.
- **What that costs us:** A small engine change (post-condition check on EndTurn against skill-declared suffix). The decisive evidence that this was the right call: DB session `03d3ec38-0839-47b6-9226-111b38d8b52b` shows the architect read its own catalogue of recurrence-1 (mika#654) on the very turn it ghosted recurrence-2. More prompt would have been more catalogue.

### Example 2 — docs-only PR exemption (mika#860 / mika#861)

- **Cheap impulse:** Define a `Pipeline-Exempt: docs-only` commit-message trailer convention. Document it in the contributing guide. "Developers will remember to add it." This is the prompt-shaped solution to a CI-shaped problem.
- **Structural correlate:** Server-side label inheritance. The linked issue's `documentation` label drives the exemption, read at check-run time via `gh api`. Labels are first-class GitHub state, observable in the PR UI, mechanically inheritable from issue → PR (no LLM judgment), and post-creation label changes are honored because the check reads at check-run time, not author-time.
- **What we shipped:** mika#861 — the label-driven check. The trailer was kept only as a residual operator escape hatch with a required-reason form, not as the primary mechanism. External pushback during design review caught this; the original draft was the trailer.
- **What that costs us:** One `gh api` call in the check workflow. Considerably less than the long-term cost of "developers will remember."

### Example 3 — Core-memory promotion protocol (mika#866 / #867 / #868)

- **Cheap impulse:** Ship the policy doc (`core-memory-as-citation-not-accumulator-2026-04-28.md`) AND a parallel prompt-level layer in the soul constants ("## Memory promotion protocol" sections) framing the prompt layer as "bridge scaffolding."
- **Structural correlate:** The policy doc itself, plus a soul.md edit moving foundational citations out of accreted core memory. No parallel prompt-level enforcement layer.
- **What we shipped:** mika#866 (policy + soul edit). mika#868's original draft included the parallel prompt layer; external reviewer cut it, citing today's own Rule 3 — shipping a prompt rule as the enforcement mechanism for a doc that warns prompt rules don't enforce. Cut rationale captured in plan A1 (`docs/plans/2026-04-28-003-feat-promotion-protocol-prompts-and-reflection-spec-plan.md`). Separately, mika#867 was filed as the /ce:review fix that should have been part of #866 — a P0 broken citation (`docs/architecture/north-star.md` → `docs/design/north-star.md`) that three reviewers flagged and that a test would have caught, had it been written first.
- **What that costs us:** Less prompt accretion. The structural enforcement (citation-test fixture for #867; the soul edit for #866) is small and bounded. The avoided cost is the ongoing maintenance of a "promotion protocol" prompt section that would have lived next to the doc forever, accumulating exceptions.

### Example 4 — Operator-side recurrence (this thread itself)

- **Cheap impulse:** Author work from conversation context. I have the canonical context already; dispatching through /mika feels like a round-trip that produces nothing the conversation hasn't already produced.
- **Structural correlate:** The canonical /mika pipeline produces a plan-on-branch artifact that downstream stages (review, work, compound) consume. Acting directly skips that artifact, and downstream stages either reconstruct it (cost) or proceed without it (silent quality loss).
- **What we shipped:** mika-platform#62 — a PR-creation CI hook that blocks source-touching PRs without a plan-doc citation. N=4 recurrences across mika#227, mika#788, mika#860, mika#866; threshold per `structural-check-replaces-human-discipline-2026-04-27.md` is N=3+. (session history) The operator-side `/mika` bypass first surfaced ~2026-04-17 in `feedback_always_use_mika.md`'s catalyst incident; today's instances are at least the second cluster of the same operator-side bias.
- **What that costs us:** One CI workflow. The avoided cost is another prompt rule in the operator soul saying "always dispatch through /mika" — a rule that would have joined the catalogue documented in `core-memory-as-citation-not-accumulator-2026-04-28.md` and not bound behavior under the next load.

### Example 5 — This very compound doc (recursive observation)

- **Cheap impulse:** Write this doc and treat it as solving the problem.
- **Structural correlate:** This doc cannot be a structural correlate. Per its own argument, it is a prompt-level catalogue entry; per Rule 3 of `required-tools-gate-evasion-patterns-2026-04-28.md`, it will not bind agent behavior under load.
- **What we shipped:** This doc, framed honestly as a citation handle for human design review — surfacing the bias as a recognizable class so future authors notice the cheap-impulse moment before shipping it. The load-bearing fixes from today's session are the structural tickets filed alongside (mika#864, mika#861, mika#866 + mika#867, mika-platform#62), not this doc.
- **What that costs us:** Honesty about scope. The doc is worth writing because human reviewers reading it during design review can recognize the pattern and route around it. It is not worth writing if it gets cited in agent prompts as authority for "the agent should ask question 1-4" — that would be the parallel-duplication failure mode it warns against, applied to itself.

## Citations

**Foundational meta-rule:**
- `feedback_prompt_enforcement_fragile.md` (mika-platform memory, written 2026-03-30) — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints." This doc is a specific instance of the meta-rule applied to a class of decisions.

**Family of prior compound docs naming the pattern in specific domains** (this doc generalizes them):
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — engine-vs-prompt for behavioral invariants
- `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` — Rule 6 second-occurrence; tagged-union tool returns
- `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — extending the same logic to human-discipline rules (skill prompt size)

**Concrete N=2 evidence:**
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 3 — "the N=2 catalogue did not prevent recurrence 2 — and the evidence is sharper than that"; pass-2 trace `03d3ec38-...`

**Sibling instances on the same 2026-04-28 thread:**
- `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — the bias applied to core-memory accretion
- `docs/solutions/best-practices/verify-which-script-ci-actually-invokes-2026-04-28.md` — the bias applied to vendored-script drift

**Methodological precedent:**
- `docs/solutions/architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md` — multi-layer structural binding (downstream of the decision this doc supports)
- `docs/solutions/prompt-enforcement-structural-guards.md` — earlier pattern doc covering post-condition early-accept + tool-side dedup

**Corollary failure mode:**
- `mika-platform/docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md` — prompt rules become fabricated-citation fodder; explicitly recommends structural allowlist over negative prompt rules

**2026-04-28 catalysts (instances cited in Examples):**
- senara-solutions/mika#788 (architect verdict-line ghost)
- senara-solutions/mika#860 (merged) + #861 (label-inheritance design)
- senara-solutions/mika#866 (merged) + #867 (merged) + #868 (in flight) — core-memory thread
- senara-solutions/mika-platform#62 — operator-side PR-creation hook

**Engine-guard tickets cited as forward-work paths** (where the structural correlates of the 2026-04-28 prompt-level instincts will land):
- senara-solutions/mika#862 — asserted-unavailability EndTurn guard
- senara-solutions/mika#863 — quoted-resource pre-fetch guard
- senara-solutions/mika#864 — required-suffix-line EndTurn guard

**Refresh candidate** (Phase 2.5 — see § "Refresh candidates" below):
- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` — pure prompt-level response with no structural backstop; warrants re-evaluation.

## Refresh candidates

The session-history scan identified `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` as the cleanest refresh candidate: it documents three prompt-only techniques (mandatory sequence markers, tool schema format requirements, iteration cap) with no structural backstop and no acknowledgment that prompt-level enforcement is fragile. This doc precedes both the 2026-04-19 engine-guards-vs-prompt-rules doc and the 2026-04-24 prompt-vs-tool-contract-mismatch doc. The new meta-doc explicitly contradicts the "harden the prompt" framing.

Other prompt-engineering docs with similar shape (lower-priority but worth flagging):
- `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md`
- `docs/solutions/prompt-engineering/2026-04-12-tighten-webhook-qa-pass-entry-point.md`
- `docs/solutions/prompt-engineering/2026-04-18-run-gh-schema-discipline-and-preflight-checks.md`

The triage isn't to delete these — they're historical records of the prompt-era fix — but each is a candidate for a "should this now be structural" review, especially the 2026-04-10 one.
