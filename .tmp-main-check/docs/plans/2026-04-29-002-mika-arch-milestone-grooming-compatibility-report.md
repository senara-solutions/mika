---
title: "Output-format compatibility report — mika-arch milestone grooming `Scope:` header"
type: compatibility-report
related-plan: docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md
related-ticket: senara-solutions/mika#879
date: 2026-04-29
status: complete
---

# Output-format compatibility report

## Why this exists

Per `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md` §4, any plan that introduces or changes an output format on a channel with documented downstream parsers must perform a parser-compatibility check before ratification. The plan at `docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md` introduces an additive `Scope: <milestone|ticket>` header line on the channel mika-arch's skills emit (consumed by `/mika-groom-ticket`, `/mika-ask-arch`, `dev-groom`, and `MIKA_ARCH_SOUL` documentation).

This report enumerates each known consumer callsite and records the verdict.

## Verdict (summary)

**No regression.** All six callsites recognize disposition/verdict tokens via **LLM-based pattern matching, not regex**. There are no anchored `^Disposition:` or line-consuming regex parsers. An additive `Scope:` line on its own line is unambiguous as long as the literal `Disposition: <KEYWORD>` final-line discipline is preserved (`docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`).

## Per-callsite analysis

| File | Lines | Pattern matched | Recognition mechanism | Verdict |
|------|-------|-----------------|----------------------|---------|
| `mika-platform/.claude/commands/mika-groom-ticket.md` | 67-69 | `Disposition: READY` / `Disposition: ITERATE` / `Disposition: ESCALATE` | Operator-LLM reads response and routes per keyword. Spec text directs the LLM to "Look for: …", not a regex. | **Unaffected** — additive `Scope:` line above the disposition does not interfere with keyword recognition. |
| `mika-platform/.claude/commands/mika-groom-ticket.md` | 88-89 | `Verdict: GROOMED` / `Verdict: ESCALATE` | Same LLM-pattern-match mechanism. | **Unaffected** — same reasoning. |
| `mika-platform/.claude/commands/mika-ask-arch.md` | 31, 33 | Documents the verdict-keyword discipline for downstream consumers (`/mika-groom-ticket`). Itself does not parse — emits `.content` verbatim + `session_id:` trailer. | Documentation-only; no parsing performed. | **Unaffected** — verbatim emission preserves whatever shape the skill produces, including the new `Scope:` line. |
| `mika/skills/bundled/dev-groom/system_prompt.md` | 41-43 | `Disposition: READY|ITERATE|ESCALATE` (in-engine equivalent of `/mika-groom-ticket`) | Skill-LLM pattern-match per its own prompt instructions. | **Unaffected** — dev-groom's input is per-ticket; it does NOT receive milestone-shaped output. The `Scope:` line never enters dev-groom's input channel in the new flow. |
| `mika/skills/bundled/dev-groom/system_prompt.md` | 56-57 | `Verdict: GROOMED|ESCALATE` | Same. | **Unaffected** — same reasoning. |
| `mika/crates/mika-agent/src/well_known_agents.rs` | 600-602 | `MIKA_ARCH_SOUL` lists the canonical disposition vocabulary (`READY, ITERATE, or ESCALATE`) for soul-level reference | Documentation in soul.md; no runtime parsing. | **Unaffected** — additive `Scope:` line is documented in the new skill's prompt, not in MIKA_ARCH_SOUL. Soul vocabulary list stays unchanged per plan D7. |

## Counter-test the analysis was tightened against

The second-pass external review challenged whether reading-the-code is equivalent to a fixture test. The challenge was right in principle: code reads can miss anchored regex hidden in helper functions or build steps.

Resolution: I grepped across `mika/`, `mika-skills/`, and `mika-platform/` for any anchored `Disposition:` or `Verdict:` regex (`grep -rE '\\^Disposition:|\\^Verdict:'`). Zero matches. Combined with the per-callsite reads above, the verdict is structural, not just textual.

Synthetic example of expected mika-arch-groom-milestone output:
```
<annotated commentary on cross-sub-issue concerns, citation-or-silence>

Scope: milestone
Disposition: READY
```

vs. existing mika-arch-groom-ticket output:
```
<annotated commentary on the per-ticket plan>

Disposition: READY
```

The only structural difference is the additive `Scope:` line. Every recognition mechanism above operates on `Disposition:` keyword presence, which is identical in both shapes.

## What would invalidate this verdict

The verdict assumes:

1. **No future PR introduces an anchored-regex parser** on the disposition/verdict channel. If one is added, this report becomes stale and the next PR touching milestone grooming must re-verify.
2. **Literal-final-line discipline holds** (per `mika-arch-first-dogfood-2026-04-25.md`). If the disposition keyword stops being the literal final line of the response — e.g., if a future skill emits `Disposition: READY\n\nSome trailing prose` — the `Scope:` placement above the disposition is unaffected, but other recognition assumptions across the platform may break.
3. **The `Scope:` line stays additive, not replacing `Disposition:`.** If a future PR proposes `Disposition: MILESTONE-READY` (replacement, not addition), the closed-alphabet assumption breaks and all six callsites need re-verification.

## References

- Plan: `docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md` (D2 decision and Unit 5 requirement)
- Discipline source: `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md` §4
- Final-line discipline: `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`
- External-review feedback that prompted this report: 2026-04-29 second-pass review (carve-out instance #3)
