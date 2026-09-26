---
title: "Operator-side findings recovery from store_fact rows when mika-arch emits thin acknowledgements"
date: 2026-04-30
category: best-practices
module: mika-arch
problem_type: workflow_issue
component: architect-grooming-flow
applies_when:
  - Running /mika-groom-ticket against any ticket with mika-arch first or second pass
  - mika-arch emits a terse final assistant message ("Persisted. Disposition: ITERATE.") referencing findings without enumerating them
  - Operator needs F1, F2, ... findings to revise the plan per Phase 4 step 11
  - mika#901 (verbatim findings emit guard) has NOT shipped yet
tags:
  - mika-arch
  - grooming-workflow
  - findings-recovery
  - thin-emission
  - disconfirmation-procedure
  - mika-901
  - mika-864
  - operator-side-recovery
  - store-fact-recovery
  - n-8-conditional-disclosure-evasion
---

# Operator-side findings recovery from store_fact rows when mika-arch emits thin acknowledgements

## Context

mika-arch's grooming skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) periodically emit thin final assistant messages — terse acknowledgements that reference findings persisted to memory rather than enumerating them in-band. The operator's `/mika-groom-ticket` Phase 4 step 11 (*"Update plan to address each architect concern"*) depends on the findings being readable from the architect's response. When emission is thin, that workflow breaks.

The architect persists detailed findings to memory via `store_fact` (with key like `mika<N>_first_pass_findings`) and/or `update_core_memory`. The findings exist; they just didn't reach the response surface. mika#901 is filed to fix this structurally (engine-side `required_finding_list` guard). Until it ships, the recovery path is operator-side query against `~/.mika/data/mika.db`.

This is N=8 of the conditional-disclosure-evasion pattern per architect's persisted `current_priorities` core memory: incidents at mika#654, #788, #863, #886, #890, #893, #874, #876. Today's session (2026-04-30) hit it twice on mika#876 (first AND second pass) and once partially on mika#901 (first-pass timed out, then succeeded clean on retry).

## Guidance

When mika-arch emits a thin response on a grooming pass:

### 1. Identify the architect session ID

The session ID appears in the `metadata.session_id` field of the `mika ask --agent mika-arch --format json --verbose` JSON response. The trailer line `session_id: <uuid>` in the printed output is the same value (re-emission for `/mika-groom-ticket` parsing). Record it for the recovery query.

### 2. Query persisted findings from `tool_calls`

```bash
sqlite3 ~/.mika/data/mika.db "
  SELECT datetime(created_at) ts, tool_name, substr(input, 1, 3000)
  FROM tool_calls
  WHERE agent_id = 'mika-arch'
    AND tool_name IN ('store_fact', 'update_core_memory')
    AND created_at >= datetime('now', '-15 minutes')
  ORDER BY created_at DESC
  LIMIT 5
"
```

Adjust the time window (`-15 minutes`) based on when the architect ran. Typical sessions are 1-5 minutes; widen the window if you don't see fresh entries.

### 3. Find the findings payload

`store_fact` calls relevant to grooming have `category: "preference"` and `key` like `mika<N>_first_pass_findings` or topic-specific patterns from the architect's persisted-memory schema (`issue_body_possible_overlap_as_phase0_gates`, etc.). The `value` field contains the F-list verbatim.

`update_core_memory` calls with `section: "current_priorities"` may carry an N-incremented entry naming the failing ticket as the latest example of conditional-disclosure-evasion — useful as cross-reference but not the findings themselves.

### 4. Apply findings to the plan as if they had been emitted in-band

The findings reach Phase 4 step 11 via the operator-side recovery rather than via the architect's response, but downstream the iteration is identical. Cite the recovery in the plan's Phase 0 Pin section so future grooms have the audit trail:

> *Findings F1–F7 from first-pass session `<uuid>` recovered operator-side from `tool_calls.store_fact` rows per the disconfirmation procedure (since mika#901 verbatim-findings emit isn't shipped).*

### 5. Cross-check against architect's persisted core memory

Read `mika-arch`'s `current_priorities` core memory for the architect's own awareness of the N-counter. If today's incident is N+1 from the prior count, the architect persisted the increment automatically — confirms the recovery is on the right session.

## Why This Matters

Cumulative cost is the load-bearing concern. Each thin-emission incident costs ~5-10 min of operator-side recovery (DB query + parse store_fact JSON + apply to plan). Across a milestone with N sub-issues each requiring 2 architect passes, the recovery cost compounds: typical 4-sub-issue milestone with 8 grooming passes × 50% thin-emission rate = ~30-40 min of pure recovery overhead per milestone.

The recovery procedure is fail-soft (operator can always retrieve findings) but not free. It's a workaround, not a fix. The structural fix is mika#901 (engine-side required_finding_list guard). Until that ships, this procedure is the canonical recovery path.

The procedure also has a meta-benefit: it forces operators to treat `store_fact` rows as authoritative artifacts rather than incidental side-effects. This builds operator literacy on the architect's persistence layer, which composes with future debugging (the same DB query path is useful for grooming-history audits, drift detection, etc.).

## When to Apply

- Apply on the THIRD line of a thin response. If the architect's final message is `Persisted. <some marker>. Disposition: ITERATE` with no F-list, recover.
- Apply when `/mika-groom-ticket` Phase 4 step 11 hits a wall — operator can't revise the plan because the findings aren't visible.
- Apply preemptively as a Phase 0 Pin if the ticket's plan-validation requires `store_fact`-backed findings (e.g., when the issue body's "Possible overlap" cross-references multiple alternatives that all need rule-out evidence — see `issue_body_possible_overlap_as_phase0_gates` preference).

Do NOT apply when:

- The architect emitted a clean response with the F-list in-band (no recovery needed).
- The architect returned READY/GROOMED with brief acknowledgement — per mika#901 issue body's acceptance criterion, brief responses are operator-spec-permitted on terminal-positive verdicts.
- The session timed out with a fallback message (`"I'm sorry, that took too long..."`). That's a different failure mode (engine 5-min deadline + MaxSteps continuation); retry the request with a tighter brief instead.

## Examples

### Bug shape — thin emission failure

mika-arch's response on mika#876 first-pass (2026-04-30 ~16:09):

```
Both patterns persisted. They generalize beyond mika#876 to any review where
brief cites a sibling ticket or the issue body names alternative causes.

Disposition: ITERATE
```

Operator's Phase 4 step 11 needs F1, F2, ... but the response has neither. Recovery query:

```bash
sqlite3 ~/.mika/data/mika.db "
  SELECT datetime(created_at) ts, substr(input, 1, 2500)
  FROM tool_calls
  WHERE agent_id = 'mika-arch'
    AND tool_name = 'store_fact'
    AND created_at >= datetime('now', '-10 minutes')
  ORDER BY created_at DESC
  LIMIT 3
"
```

Returns two `store_fact` rows with keys `issue_body_possible_overlap_as_phase0_gates` and `issue_body_cross_reference_vs_brief_cited_evidence`. Each `value` contains a finding's full text — operator unwraps and treats as F1 + F2 of the response. Applies to plan; commits revisions; sends second-pass brief.

### Counter-example — clean emission, no recovery needed

mika-arch's response on mika#904 first-pass (2026-04-30 ~16:13):

```
Ground truth verified... [several lines of evidence citation] ...

## Architect First-Pass Review — mika#904

### Findings

F1 (BLOCKING) — Causation test "iteration-history trace" lacks operational definition...
[full F1-F7 enumerated with concern + change required + citation]

Disposition: ITERATE
```

F-list is in-band. Operator skips recovery and proceeds directly to Phase 4 step 11.

### Verification — confirm the recovered findings match the persisted reasoning

Architect's `current_priorities` core memory (read via the same query, filtered to `update_core_memory`) typically carries an N-counter increment for thin-emission incidents:

```
Required-tools-gate evasion (N=8): #654/#788/#863/#886/#890/#893/#874 prior;
#876 1st-pass 2026-04-30 — thin acknowledgement emission ... exactly the
failure class #901 is filed to fix; findings recovered operator-side from
store_fact rows.
```

If today's incident appears as the latest N-increment, the recovery is on the right session. If not, double-check the session_id and time window.

## Related

- mika#901 — structural fix (engine-side `required_finding_list` guard analogous to mika#864's `required_suffix_lines`); shipping closes the recovery-cost source
- mika#864 — `required_suffix_lines` guard precedent that mika#901 mirrors
- `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — the broader N=8 pattern this is an instance of
- `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the ratchet precedent (3 instances codifies; we're at 8)
- `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` — emission contract source (today's prompt-only enforcement that drifts)
- `mika/skills/bundled/mika-arch-second-review/system_prompt.md` — same surface, second-pass shape
