---
title: "chore: Delete propagated mika-groom-milestone.md command copy"
type: refactor
status: active
date: 2026-05-12
origin: ../../../mika-platform/docs/plans/2026-05-12-002-chore-delete-propagated-groom-milestone-plan.md
---

# chore: Delete propagated mika-groom-milestone.md command copy

## Overview

Delete the propagated copy of `mika-groom-milestone.md` from `mika/.claude/commands/` to match the `mika-groom-ticket.md` precedent (canonical-only-in-meta-repo). Update the smoke test to remove the section that validates the now-deleted file.

## Problem Frame

mika#888 shipped a propagated copy of `mika-groom-milestone.md` into `mika/.claude/commands/`. This diverges from the established precedent: `mika-groom-ticket.md` exists only at `mika-platform/.claude/commands/` with no propagated copy. Two copies create a drift surface with three structural divergences (SCRIPTS_DIR resolution, doc path prefixes, dispatch wording) and no structural enforcement. See `docs/solutions/best-practices/canonical-propagated-prose-pair-discipline-2026-04-29.md` § 2.

## Requirements Trace

- R1. Delete `mika/.claude/commands/mika-groom-milestone.md` (the propagated copy)
- R2. Update `scripts/test-mika-groom-milestone.sh` to remove section 3 ("Operator command") and renumber subsequent sections
- R3. Canonical copy at `mika-platform/.claude/commands/mika-groom-milestone.md` remains unmodified

## Scope Boundaries

- No behavioral changes to milestone grooming
- No changes to the canonical copy in mika-platform
- No changes to the bundled skill scaffold, well_known_agents.rs, sequencing template, or compatibility report

## Key Technical Decisions

- **Delete over fix:** Deleting the propagated copy (matching `mika-groom-ticket.md` precedent) is strictly better than maintaining two copies with a transformation table. Operators invoke milestone grooming from meta-repo cwd where the canonical copy resolves.

## Implementation Units

- [ ] **Unit 1: Delete propagated command file and update smoke test**

**Goal:** Remove the drift surface by deleting the propagated copy and updating the smoke test.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Delete: `.claude/commands/mika-groom-milestone.md`
- Modify: `scripts/test-mika-groom-milestone.sh`

**Approach:**
- Delete `.claude/commands/mika-groom-milestone.md`
- In `scripts/test-mika-groom-milestone.sh`:
  - Remove section 3 ("Operator command") entirely (lines 81–89)
  - Update the header comment: remove item 3 ("The operator command exists"), renumber items 4→3, 5→4, 6→5
  - Renumber section headers in the script body: "4. Sequencing record template" → "3.", "5. Compatibility report" → "4.", "6. Review guide codification" → "5."

**Patterns to follow:**
- `mika-groom-ticket.md` — canonical-only-in-meta-repo shape (no propagated copy in mika/)

**Test scenarios:**
- Happy path: `bash scripts/test-mika-groom-milestone.sh` passes with all remaining checks green
- Edge case: Verify no other files reference `.claude/commands/mika-groom-milestone.md` as a functional dependency

**Verification:**
- `.claude/commands/mika-groom-milestone.md` no longer exists in the mika repo
- `bash scripts/test-mika-groom-milestone.sh` passes
- Canonical copy at `mika-platform/.claude/commands/mika-groom-milestone.md` is untouched

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Operator invokes `/mika-groom-milestone` from mika/ cwd | Not a real risk — milestone grooming requires cross-repo scripts that live in mika-platform/. Same model as mika-groom-ticket. |

## Sources & References

- Origin plan: `mika-platform/docs/plans/2026-05-12-002-chore-delete-propagated-groom-milestone-plan.md`
- Compound: `docs/solutions/best-practices/canonical-propagated-prose-pair-discipline-2026-04-29.md`
- Source PR: mika#888 (shipped the propagated copy)
- Canonical add PR: mika-platform#65
- Related issue: mika-platform#66
