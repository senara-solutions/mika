---
module: agent-core, dev-loop
tags: [engine-guard, structural-enforcement, grooming, mika-arch, disposition, fail-closed, attestation]
problem_type: bug-class-prevention
category: best-practices
component: tooling
severity: high
applies_when:
  - A guard family enforces a contract on some outcomes of an enum and exempts the others
  - A keyword emitted by a model is parsed downstream as an attestation that work was done
  - A guard's refusal is consumed by a second layer that does its own, looser matching
---

# The exempted half of a guard is where the next failure lands

## Problem

mika#901 added an engine guard: when mika-arch returns a **terminal** disposition
(`ITERATE` / `ESCALATE`), the response must carry an F-list of findings. Non-terminal
dispositions — `Disposition: READY`, `Verdict: GROOMED` — were exempted by design, and the
skill prompt said so in as many words: *"On READY, the F-list is NOT required — the message
may stay short since no iteration is needed."* The reasoning was sound on its face: a plan
with no objection has no findings to emit.

On 2026-08-29 mika-arch returned, on a 10 492-byte brief carrying four numbered questions,
this complete response (302 bytes, 114 seconds, session `d6411163-6bc0-47d9-b642-954adf8d3f64`):

> Préférence stockée — le pattern de re-résolution par cycle et le seuil N=3 pour mika#2013.
>
> Disposition: READY

None of the four questions addressed. No code cited. A decision hallucinated — the brief said
literally *"I have not fixed N"* and the response asserts a threshold of N=3. And the keyword
emitted anyway.

`/mika-groom-ticket` Phase 3 step 10 parses `Disposition: READY` and commits the plan as
architect-validated. The whole staging-before-commit discipline exists so the first commit on a
grooming branch is signed by a review. **The exempted disposition was the only one that advances
the chain, and the only one owing no evidence.** Every model that sits in that seat inherits the
hole; mika#1957 is the same class at n=2, where a first-pass `READY` shipped a plan carrying two
false premises.

## Solution

**Enumerate the outcomes your guard exempts, and ask what each exempted one authorizes.** A
guard family that covers the outcomes which *report a problem* while exempting the outcomes
which *let work proceed* has it exactly backwards: the permissive branch is the one an
unearned answer wants to reach. The fix is not to widen the existing contract — requiring an
F-list on READY would push the model to fabricate findings — but to give the exempted half its
own contract, of a shape appropriate to it.

**Require something only the real work can produce.** Three candidates were weighed for
attesting a review:

| Candidate | Why it fails or holds |
|---|---|
| Answering the brief's numbered questions | Depends on the brief being numbered. Free-text briefs are not, so the guard goes inert exactly where it is needed. |
| A per-criterion verdict | Trivially satisfiable by a grid of "OK" with no content. |
| **Verbatim quotation of the brief** | The engine already holds the brief. A quote is checkable mechanically, against a source that exists, with no judgment call. |

The bite comes from **dispersion, not volume**. One quote is crossable by copying the brief's
first line. Three quotes at non-overlapping positions of a multi-kilobyte document are not a
by-product of an acknowledgement — they require having moved through it.

**Make this half fail-closed even when its siblings are fail-open.** The rest of the guard
family re-prompts once and then accepts; that is right for a contract whose violation is a
formatting slip. It is wrong here, because accepting after two failures restores the exact hole
being closed. A response the engine cannot validate as a review is **an absence of verdict, not
an approval** — so the disposition line is stripped from the final text and replaced with a
literal marker, with the model's prose kept intact. Nothing is lost but the attestation that
was not earned.

**A refusal is only as strong as the consumer that honors it.** This is the part that prose
review would have missed. Removing the disposition line is not enough: `dispatch-lib`'s
`_parse_disposition` has three tiers, and tier 2 matches paraphrases (`proceed`, `good to go`,
`plan is clean`) *anywhere in the text*. A response whose disposition line was merely deleted
could still yield `READY` out of its own body. The consumer therefore short-circuits on the
marker **before tier 1a** — and the position is the whole point. After tier 1a the marker would
still be honored; after tier 2 it would not. Two of the shell assertions exist specifically to
fail if tier 0 ever moves down.

## Evidence

- mika#2037 body — the measured response, its session id, and the brief it answered.
- `crates/mika-agent/src/agent_loop/mod.rs` — `is_terminal_disposition()` is the exemption; the
  review-anchor guard immediately after the mika#901 guard is its complement.
- `crates/mika-agent/src/agent_loop/review_anchor.rs` — the matcher, and `mod matrix`, the
  two-column measurement.
- **Thresholds are measured, not chosen.** The sweep reports nine separating
  `(min_count, min_quote_chars)` pairs, from `(2,24)` to `(3,56)`; the shipped `(3, 40)` sits
  in the middle of that region rather than on its edge. `min_count = 1` separates nothing —
  which is the measurement behind choosing 3.
- **Two design defects found by measurement, not by reading.** (1) The anchor prefix (`A1: `)
  was being counted inside the quote window, so a longer prefix bought a shorter quote — the
  declared 40 characters were really 36 of content. (2) Whitespace needed normalizing on both
  sides: a reviewer re-flows what they quote, so a genuine citation was failing on the *brief's*
  line wrapping. Both were invisible to prose review and immediate under a case table.

## When to apply

- Any guard keyed on an enum where only some variants carry the obligation. Write down what the
  exempted variants let through.
- Any keyword a downstream consumer treats as evidence that work happened. If the keyword is
  cheaper to emit than the work is to do, it will eventually be emitted without the work.
- Any engine-side refusal consumed by a second layer that does its own matching. Grep the
  consumer for looser tiers before assuming the refusal survives the trip.

## What this deliberately does not do

**It does not target the model.** `openrouter/moonshotai/kimi-k2.5` produced that response that
day; the defect is that no guard caught it. A fix that swapped the model would leave the door
open for the next one, and the guard carries no model, provider, or seat identifier anywhere in
its code, configuration, or tests.

## Related

- `required-finding-list-guard-conditional-disclosure-evasion-2026-05-13.md` — the guard whose
  exemption this closes; the same manifest-declared, engine-enforced pattern.
- `groomed-plan-is-a-shape-contract-not-a-fact-contract-2026-08-27.md` — n=2 of the class, and
  the source of the discipline that a gate's matcher gets a case matrix before it goes into the
  file.
- `required-suffix-line-guard-verdict-ghosting-structural-fix-2026-04-29.md` — the founding
  member of the family, and the anti-regex precedent this matcher follows.
- `feedback_prompt_enforcement_fragile` — why the prompt change alone was not the fix, and why
  the prompt change was still necessary: it was actively teaching the defect.
