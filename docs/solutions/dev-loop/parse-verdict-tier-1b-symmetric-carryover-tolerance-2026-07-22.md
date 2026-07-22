---
module: dev-loop, dispatch-lib, architect-second-pass
tags: [session-carry-over, verdict-parser, symmetric-fix, orphan-half, self-mod-invariant]
problem_type: parser-contract-gap
category: dev-loop
date: 2026-07-22
ticket: mika#1819
applies_when:
  - "Any autonomous groom loop where a two-pass parser (first-pass produces one keyword shape, second-pass produces another) shares a session with the LLM producer"
  - "You already have a tolerance patch for one direction of the asymmetry (e.g. mika#1421 v3 for _parse_disposition) and are diagnosing loop stalls on the mirror class"
resolution_type: pattern
---

# `_parse_verdict` tier 1b — symmetric carry-over tolerance (orphan-half fix)

## TL;DR

`_parse_verdict` in `skills/bundled/_shared/dispatch-lib.sh` accepted only strict `Verdict: GROOMED|ESCALATE`. When mika-arch reused a session across dispatches and legitimately declined a third-pass on an unchanged plan, it emitted first-pass shape `Disposition: READY` instead. The parser fell through Tier 1 → Tier 2 fuzzy → default `_escalate_groom`, producing spurious ESCALATE verdicts on plans the architect had already ratified.

Root cause: `_parse_disposition` already carried the symmetric fix (mika#1421 v3, tier 1b accepting `Verdict: GROOMED → READY` on session-recall). **`_parse_verdict` was the orphan half — the mirror was never implemented.** Fix: +15 lines adding tier 1b to `_parse_verdict` accepting `Disposition: READY → GROOMED` and `Disposition: ESCALATE → ESCALATE`.

## Founding streak (evidence hard)

Between 2026-07-01 and 2026-07-22, `audit_events` shows **16+ ESCALATE architect-second-pass-after-ready events on mika-dev across 8+ distinct tickets, spanning 2 repos**:

| Date | Ticket | Repo |
|---|---|---|
| 2026-07-01 21:35 | mika#1716 | mika |
| 2026-07-02 07:44/07:45/08:02/08:03 | (unnamed), mika#1667, mika#1627, mika#1664 | mika |
| 2026-07-04 18:03 | mika#1723 | mika |
| 2026-07-06 09:36/09:43/11:49/11:54/12:01/12:10 | mika-cloud#139, mika-cloud#128 (2×), mika-cloud#140, mika#1727 | both |
| 2026-07-22 12:45/12:59 | mika#1716 (re-kick) | mika |

All plans passed content grooming; the architect confirmed prior-turn ratification in every case. Verified across 4 branches via `gh api` on their `.iterate/escalate-second-pass-after-ready.md` files — every one ends with `Disposition: READY` (first-pass shape).

Amplification post-per_spawn: the 2026-07-22 cpp per_spawn merge + `.iterate/` purge cleanups forced mika-arch to re-evaluate branches that were previously masked by stale artifacts. Result: every fresh evaluation triggered the parser gap, and the ~23 ready backlog dammed on one class alone.

## Why mika-arch emits the "wrong" shape

She's honest — not dysfunctional. On session-recall of an unchanged plan, she recognizes the ratification-in-prior-turn and declines a third pass per `mika-arch-second-review/system_prompt.md` spec §4.5 / R11 ("two-pass limit"). She emits `Disposition: READY` as ratification-of-recall, not `Verdict: GROOMED` (which would imply a fresh review actually happened). The bug is in the PARSER, not the architect.

## Fix

`_parse_verdict` in `skills/bundled/_shared/dispatch-lib.sh` — add tier 1b between existing Tier 1 (strict `Verdict:`) and Tier 2 (fuzzy fallback):

```bash
# Tier 1b — session-carry-over tolerance (symmetric to _parse_disposition
# mika#1421 v3 which accepts Verdict: → READY).
local disposition_keyword
disposition_keyword=$(printf '%s' "$text" \
    | grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
    | grep -oE '(READY|ITERATE|ESCALATE)' \
    | head -1)
case "$disposition_keyword" in
    READY)
        echo "GROOMED"
        return
        ;;
    ESCALATE)
        echo "ESCALATE"
        return
        ;;
    # Deliberately no ITERATE case — spec §4.5 R11 forbids third-pass.
esac
```

## Why symmetric matters

The two parsers are contract mirrors of each other:

| Function | Called after | Session-recall risk | Symmetric tolerance |
|---|---|---|---|
| `_parse_disposition` | mika-arch first-pass | Architect emits second-pass shape (`Verdict: GROOMED`) if prior session already reached second-pass | tier 1b: `Verdict: GROOMED → READY`, `Verdict: ESCALATE → ESCALATE` (mika#1421 v3, shipped 2026-06-06) |
| `_parse_verdict` | mika-arch second-pass | Architect emits first-pass shape (`Disposition: READY`) if prior session already ratified without a third pass | tier 1b: `Disposition: READY → GROOMED`, `Disposition: ESCALATE → ESCALATE` (mika#1819, this doc) |

Asymmetric tolerance = orphan half. **If one direction ships without the mirror, the loop stalls on the class the mirror would have caught, but the failure mode is silent (default-escalate looks legitimate).** The mika#1421 v3 fix took 6 weeks to reveal its own orphan half because per_spawn was masking the streak with a different failure upstream.

## When to reach for this pattern

- You've added session-carry-over tolerance to one function in a bidirectional contract (input shape A→B or B→A depending on session state)
- The other function will need the mirror — **the same session-recall pathology applies symmetrically**
- If you don't add the mirror concurrently, expect a silent-failure loop-stall class to appear later, framed as a different problem

## What we do NOT do here

- **Do NOT change the mika-arch prompt** to force `Verdict: GROOMED` on session-recall. The current architect behavior (declining third-pass, emitting first-pass ratification shape) is spec-correct per §4.5 R11. The parser must tolerate the spec, not the prompt reshape the emission.
- **Do NOT map `Disposition: ITERATE` to any verdict.** Spec explicitly forbids a third-pass. ITERATE at second-pass falls through to Tier 2 fuzzy, which is typically ESCALATE-biased (safe default).
- **Do NOT auto-merge this class of fix.** It touches `dispatch-lib.sh` — substrat-de-décision-de-merge. Self-mod invariant (ratified coherence-architect × Prime, 2026-07-22): the loop MUST NOT auto-merge its own verdict parser. **Vincent merge-admin only.**

## References

- `mika#1421 v3` — the sibling fix for `_parse_disposition` this mirrors
- `mika#1819` — this PR
- `mika-arch-second-review/system_prompt.md § 4.5 / R11` — the spec that motivates the architect's session-recall behavior
- `dispatch-lib.sh:_parse_disposition` (line 1706) — the existing tier 1b to mirror against
- Founding streak audit_events query: `SELECT created_at, target_key, reasoning FROM audit_events WHERE agent_id='mika-dev' AND tool_name='update_task_status' AND reasoning LIKE '%second-pass-after-ready%ESCALATE%' ORDER BY created_at DESC`
