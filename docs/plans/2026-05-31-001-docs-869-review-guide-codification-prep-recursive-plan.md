# Plan: docs(review-guide) codification-prep — recursive-self-review carve-out (mika#869)

**Ticket:** senara-solutions/mika#869
**Type:** documentation
**Status:** Close-ready — all acceptance criteria met by prior work

---

## Finding: work already shipped

This ticket was opened at N=2 as **prep work** for codifying the recursive-self-review carve-out into `docs/architecture/review-guide.md` when the third instance (N=3) arose.

N=3 has already occurred and the codification has already shipped:

- **N=3 instance:** mika#879 (milestone-grooming additions to mika-arch bundled skills) — CLOSED.
- **Codification commit:** `34e0558e` (`feat(skills/mika-arch): add milestone-grooming sibling skill + operator command (#879)`) added §7 "Self-review boundary" to `review-guide.md`.
- **Follow-up refinements:** `70d0f5cf` (mika#904, causation-vs-outcome-shape sharpening) and `a83d831f` (mika#921, further sharpening of the provenance test + memory-shared axis).

### Acceptance criteria verification

| Criterion | Status | Evidence |
|-----------|--------|----------|
| N=3 instance arises | ✅ Met | mika#879 (CLOSED) — third exercise of the carve-out |
| `review-guide.md` gains "self-review boundary" subsection | ✅ Met | §7 exists at lines 168–225, cites #818, #868, #879, #874 |
| Criterion-#1 empirical evidence cited | ✅ Met | §7 "Evidence base" references all four instances |
| This ticket closed with a comment linking the N=3 PR | ❌ Pending | Ticket still open; needs closing comment |

### What §7 contains (already shipped)

The codified §7 covers everything this ticket's scope items anticipated:

1. **Convergence/divergence assessment (scope item 1):** §7 affirms the mechanism without prescribing intervention style — exactly the finding from this ticket's body.
2. **Criterion-#1 empirical justification (scope item 2):** §7's evidence base includes mika#868 (criterion-#1 trigger) and has been refined with mika#874's causation-vs-outcome-shape analysis (the sharpened two-gate test).
3. **Codification language (scope item 3):** §7 contains the full codification — trigger conditions, provenance test, what-to-flag/what-not-to-flag, memory-shared coupling analysis, and future-work note on memory-key namespacing.

Additionally, §7 went beyond this ticket's anticipated scope with two post-codification refinements:
- mika#904: causation-vs-outcome-shape sharpening (three-state taxonomy, provenance gate)
- mika#921: memory-shared isolation axis + §6/§7 division clarity

## Recommended action

**Close mika#869** with a comment linking the N=3 PR (mika#888) and noting that §7 codification shipped at `34e0558e`, with subsequent refinements at `70d0f5cf` (mika#904) and `a83d831f` (mika#921).

No code or documentation changes required — the prep work this ticket anticipated has been superseded by the actual codification.

## Implementation steps

1. Post a closing comment on mika#869 citing the shipped artifacts:
   - N=3 instance: mika#879
   - Codification PR: mika#888 (commit `34e0558e`)
   - Refinements: mika#904 (commit `70d0f5cf`), mika#921 (commit `a83d831f`)
   - §7 location: `docs/architecture/review-guide.md` lines 168–225
2. Close the issue.

## Risk assessment

**None.** This is a close-as-done ticket with no remaining work.
