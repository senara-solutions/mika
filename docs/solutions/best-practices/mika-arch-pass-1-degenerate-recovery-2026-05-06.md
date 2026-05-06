---
title: mika-arch pass-1 degenerates on multi-thread reconciliation; retry with fresh session before escalating
date: 2026-05-06
category: best-practices
module: mika-arch
problem_type: best_practice
component: skills
severity: medium
applies_when:
  - Running /mika-groom-ticket and the architect's first-pass response is degenerate (3-5 line echo, "Disposition: ESCALATE" with no reasoning, or otherwise non-substantive)
  - The grooming question requires multi-thread reconciliation — cross-referencing prior decisions, GROOMED plans, body edits, or sibling tickets
  - Operator is wondering whether the degenerate response means escalation is genuinely warranted vs. a pass-1 reliability glitch
tags:
  - mika-arch
  - mika-groom-ticket
  - architect-roundtrip
  - reliability
  - retry-pattern
  - operational-discipline
---

## Context

mika-arch's two-pass review (mika-arch-groom-ticket → mika-arch-second-review) is the architect roundtrip the autonomous loop relies on for plan validation. On 2026-05-06 across one extended grooming session, three reconciliation-class questions were posed to mika-arch. **Two of three pass-1 responses were degenerate** — short, non-substantive, no review content (e.g., "Disposition: ESCALATE" as the entire body, no findings, no reasoning). All three were resolved by retry-with-fresh-session, with the retry producing coherent, actionable reviews that correctly identified spec divergences and structural concerns.

The ratio (2/3) is calibration data, not a deterministic pattern. Earlier sessions (per the 2026-05-06 handsoff log on mika-platform#81) showed the same shape — degenerate first attempt, retry succeeds. The pattern correlates with **multi-thread reconciliation questions**: questions where the architect must reason against prior GROOMED plans, body-edit history, or sibling-ticket constraints to evaluate the current ticket.

## Guidance

When mika-arch pass-1 returns a degenerate response on a /mika-groom-ticket call:

1. **Do NOT immediately escalate to operator.** A degenerate response is not a substantive ESCALATE — it's a pass-1 reliability glitch.
2. **Retry once with a fresh session.** Use the same `mika ask --agent mika-arch --format json --verbose` invocation, but do NOT pass `--session-id` (or pass a fresh session id). The retry should load the brief content as if it were a first encounter.
3. **The brief itself is unchanged.** No need to rewrite — the reliability glitch is in mika-arch's session priming, not in the brief's content.
4. **If the retry ALSO degenerates on the same plan**, that's an operator-level escalation. Surface to the operator with both responses concatenated; flag as a third instance for `project_mika_arch_failure_modes.md` calibration record (per the working-note compound on mika-arch failure modes).
5. **The pattern does NOT mean "always retry once."** Substantive ESCALATE responses (with findings, reasoning, citations) are real outcomes — apply spec discipline (V1/V2/V3 resolution paths) per the operator's protocol. Only retry when the response is observably non-substantive.

**Heuristic for "degenerate":** the response is shorter than ~10 lines, contains no specific findings (no F-numbers, no file paths, no citations), and the "Disposition" line is the entire body. A response with one finding but terse phrasing is NOT degenerate — that's a real verdict.

## Why this matters

Without this pattern, operators interpret degenerate pass-1 responses as genuine ESCALATE outcomes and either (a) halt the entire grooming flow prematurely, or (b) escalate trivial questions to the operator that the architect could have handled on retry. Either path wastes operator attention and slows the autonomous-loop cadence the structural fixes from this session (mika#988/#996/#991/#1001) are designed to enable.

The retry cost is negligible (~30s extra wall-clock per occurrence). The escalation cost is meaningful (operator attention, plan-flow disruption, potential premature scope-narrow). Retry-first is the dominant strategy.

## When to apply

- Inside `/mika-groom-ticket` when calling `/mika-ask-arch` and the response trips the degenerate heuristic.
- Inside `/mika-ask-arch` directly when the JSON envelope's `.content` field is observably non-substantive.
- During multi-pass groom flows (pass-1 OR pass-2) — the pattern applies symmetrically; pass-2 degeneration is rarer but uses the same retry mechanism.

Do NOT apply to:
- Substantive ESCALATE responses (those have real findings worth surfacing).
- Substantive ITERATE responses (pass-1 ITERATE is the spec's normal path; retry would discard valid feedback).
- Other architect skills outside the groom-ticket family unless reliability data accumulates for them too.

## Examples

**Instance 1 (mika-platform#81, 2026-05-06 overnight per handsoff log):** pass-1 returned a 3-line response with no review content. Retried once with fresh session. Pass-1 retry produced a coherent review identifying real findings. Plan proceeded to GROOMED on pass-2.

**Instance 2 (mika#996, this session):** pass-1 returned `Disposition: ESCALATE` as the entire body. Retried once with fresh session. Pass-1 retry produced a substantive ESCALATE with three legitimate concerns (E1 AC#4 deferral, E2 dev-groom operator-only restriction, E3 engine-guard contradiction) — all genuinely required operator judgment. The first response was a glitch; the retry was the real verdict.

**Instance 3 (mika#1001, this session):** pass-1 was substantive on first attempt (NOT degenerate). Pass-2 caught a ticket-body misframing (E1) on first attempt as well. Counter-evidence that degenerate-pass-1 is NOT deterministic — sometimes mika-arch handles reconciliation cleanly. The retry pattern is recovery, not a default cycle.

**Pattern-level inference:** the degenerate response correlates with how much prior context the question requires reconciling. mika#1001's pass-1 brief was tightly scoped (Option A/B/C with concrete shapes). mika#996's pass-1 brief asked the architect to reason against an existing GROOMED plan AND a recent body edit — more reconciliation surface. The fix on retry is the same regardless of which pass produced the glitch.

## Related

- `project_mika_arch_failure_modes.md` (working note in operator's auto-memory) — tracks accumulated mika-arch failure modes including criterion-replacement, deadline-timeout, contract-fabrication. This compound adds the degenerate-pass-1 reliability mode as a fourth recorded failure class.
- `mika/docs/solutions/best-practices/autonomous-agent-operational-discipline-2026-04-23.md` (sibling) — broader operator discipline patterns this fits within.
- `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` — the skill that produces the responses; future hardening work could include a self-check that rejects own-degenerate responses before sending.
