---
module: development_workflow
date: 2026-05-06
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Diagnosing a "broken again" infrastructure failure (CI, release automation, deploy)
  - Drafting a peer-review brief or proposing a non-trivial fix in a documented domain
  - Operating any pipeline step that's intended to consume institutional memory
tags:
  - infra-fix
  - compounding
  - institutional-memory
  - rule-application
  - diagnostic-discipline
  - chronic-drift
related_components:
  - tooling
  - documentation
related:
  - docs/solutions/best-practices/infra-fix-compounding-practice-2026-04-23.md
  - docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md
---

## Context

On 2026-04-23, mika#776 institutionalized a rule via `feedback_compound_infra_fixes.md`: *"infra fixes evaporate faster than product fixes; compound every non-trivial one, look back for prior related fixes before shipping a new one."* The rule's case-study compound doc (`infra-fix-compounding-practice-2026-04-23.md`) names a specific corollary — *"look back before shipping"* — and lists release automation chronic drift as the originating evidence.

On 2026-05-06, twenty-one days after that rule was institutionalized, an operator session opened with the prompt *"the release process is broken again."* The agent (me) immediately drafted a fresh diagnosis ("homegrown bash, should migrate to upstream `release-plz`") and produced a peer-review brief proposing the migration. The brief was fluent, evidence-cited, and **wrong in a way the look-back rule was specifically designed to prevent**:

- mika#775 was already open with this exact symptom and three approaches enumerated.
- `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` had already classified it as Class C and noted Class A had killed the upstream `release-plz` tool eight weeks earlier.
- The open ticket explicitly listed *tool switch* as out-of-scope unless investigation showed the chosen approach couldn't work within Class A/B/C constraints.

The peer caught the contradiction in the very first review pass. The fix that landed honored the existing ticket; no migration. The rule worked — but it worked **post-hoc** (peer review caught the agent), not **pre-hoc** (the agent applied the rule on first contact).

This document captures what that gap implies for how look-back rules should be operationalized.

## Guidance

**A documented rule is a half-step. The rule's full form is the structural primitive that makes "did you check?" unmissable on first contact.**

For any failure surface where a chronic-drift class doc exists in `docs/solutions/`, the first move on a "broken again" symptom is **not** drafting a fresh diagnosis. The first move is a three-command pre-flight:

```bash
# 1. Search for class-level prior art in the matching solutions category
ls docs/solutions/<category>/ | grep -i <surface-keyword>

# 2. Search for open tickets with the same symptom
gh issue list --repo <repo> --state open --search "<symptom-keyword>"

# 3. If a workflow file is implicated, check its migration history
git log --oneline --follow .github/workflows/<file>
```

If any of those surface a class doc or open ticket, the diagnosis is already partially done — adopt it as the starting point, don't re-derive it.

**Calibration for "when does this rule apply":**

| Surface signal | Pre-flight? |
|---|---|
| User says "broken again" / "regression" / "recurring" | **Yes** — this is the literal trigger phrase for the rule |
| File path matches `.github/workflows/*`, `Dockerfile*`, `Makefile`, deploy scripts | **Yes** — historically high chronic-drift density |
| Symptom is in an area with `docs/solutions/` documentation | **Yes** — the cost of the pre-flight is ~30 seconds |
| Net-new feature in unfamiliar code | No — the rule is about *recurrence*, not first-pass research |

**The rule fires structurally if these checks happen before any analysis longer than two tool calls.** "Two tool calls" is the operational definition: if a brief, plan, or PR description requires more than two tool calls' worth of reasoning, the pre-flight should already have run. Anything past that is committed effort and the cost of redirecting compounds.

## Why This Matters

The look-back rule's institutional weight is not "don't make this mistake." It's "don't make this mistake **twice**." mika#776 codified the first instance (3 tools, 14+ fix commits over 7 weeks); this session is a second instance, with the rule itself as the trigger. The pattern that emerges:

| Layer | Cost when rule fires | Cost when rule misses |
|---|---|---|
| Memory entry exists | ~30s pre-flight | Wasted brief (5–15min), wasted peer turn (5–10min), occasional mis-routing |
| Compound doc exists | Same | Brief proposes a strategy already explicitly out-of-scope on the open ticket |
| Open ticket exists | Adopted directly | Contradicts the ticket's stated approach without realizing it |

The arithmetic is asymmetric: the pre-flight is cheap regardless of outcome. The miss is expensive in proportion to how well-documented the surface already is. **A surface with high prior-art density is the *most* dangerous one to free-form diagnose**, because the agent's confidence rises with the surface's familiarity even though the right answer is "stop and read."

There's a second-order effect: rules that fire only post-hoc accumulate a reputation as "advisory" rather than "operational." Each post-hoc catch is evidence to future readers that the rule didn't fire pre-hoc — making the rule itself feel optional. Each pre-hoc fire is evidence the rule is load-bearing. Compounding works; it just requires the rule to be applied at the moment it's relevant, not after the fact.

## When to Apply

- **Always**, on any task whose user-facing prompt contains a recurrence signal ("broken again", "regression", "this keeps happening", "back to square one"). No judgment call — the prompt itself is the trigger.
- **Always**, before drafting a `/mika-ask-a-friend` brief on an infrastructure topic. If the brief is going to argue for a strategy or tool migration, the prior-art search is part of the brief's evidence base, not optional context.
- **Always**, in the dispatcher's pre-flight gates (e.g., the WHY-first principle in `/mika`, `/mika-groom-ticket`'s evidence-only constraints). If a future structural primitive is added, this is the surface to add it on — not as a memory entry that the agent has to remember to consult, but as a check the dispatcher runs by default.

The rule is *not* meant for net-new feature work in unfamiliar areas — that's a different discipline (research-then-design). It's specifically for recurrence in documented surfaces, where the prior art is the answer.

## Examples

**Pre-flight that didn't run (this session, 2026-05-06):**

```
User: "you need to fix the release process. it's broken again."
Agent: <reads workflow file, runs gh run list, drafts diagnosis>
Agent: <writes peer-review brief proposing tool migration>
Peer: "Halt — your problem statement and project ground truth disagree.
       mika#775 is already open with this exact symptom and three approaches
       enumerated. Tool switch is explicitly out-of-scope on the ticket because
       Class A killed release-plz in April."
Agent: <reads chronic-drift doc, mika#775 body, mika#776 body>
Agent: <acknowledges error, adopts the open ticket's approach, dispatches>
```

Net cost of the miss: one peer turn (~10 minutes), one brief draft (~10 minutes), one calibration memory write. The rule worked, but worked late.

**Pre-flight that did run (counterfactual, what should have happened):**

```
User: "you need to fix the release process. it's broken again."
Agent: <runs three-command pre-flight>
       ls docs/solutions/ci-cd/                       # surfaces release-automation-chronic-drift-2026-04-23.md
       gh issue list --repo .../mika --state open --search "release"
                                                      # surfaces mika#775
       git log --oneline --follow .github/workflows/release-plz.yml
                                                      # surfaces 4825e7ae migration
Agent: "I see mika#775 is open for this. Reading the chronic-drift compound doc
        and the ticket body for the documented approach, then dispatching."
```

Net cost of the hit: ~30 seconds. Net cost of the miss: 20+ minutes plus the institutional cost of validating that the rule still works.

**Symmetry counterexample — when the rule does not apply:**

```
User: "Add a new health endpoint to mika-spirit."
Agent: <does not run the pre-flight; this is net-new work in a documented area
        but not a recurrence>
Agent: <reads existing endpoints, drafts plan, proceeds normally>
```

The rule's trigger is *recurrence*, not *documented area*. Net-new feature work doesn't qualify even when the surface has compound docs.

## Cross-references

- [`infra-fix-compounding-practice-2026-04-23.md`](./infra-fix-compounding-practice-2026-04-23.md) — the originating rule (mika#776). This doc is the half-step beyond it: the rule's structural form, not just the rule's statement.
- [`release-automation-chronic-drift-2026-04-23.md`](../ci-cd/release-automation-chronic-drift-2026-04-23.md) — the case study. Contains four failure classes (A/B/C/D) and Stage 3 (Class C resolution) added by mika#775.
- MEMORY: `feedback_compound_infra_fixes.md` — the rule in user-memory form. mika#776 institutionalized this entry.
- MEMORY: `feedback_search_solutions_before_diagnosing_ci.md` — calibration memory written immediately after the peer caught the framing error in this session. The lower-tech version of this doc's structural pre-flight.
- MEMORY: `feedback_evidence_before_diagnosis.md` — adjacent rule on querying state before proposing fixes. Same family of "look first, propose second" disciplines.
