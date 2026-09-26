---
ticket: mika#1421
type: fix
component: skills/bundled/_shared/dispatch-lib.sh
date: 2026-06-06
seq: 003
base_sha: e1ebebc3
related:
  - mika#1381  # n=1 architect-convergence observation
  - mika#771   # n=2 architect-convergence binding (founding incident)
  - mika#1271  # dispatch-lib content/workflow split (introduced _iterate_groom_loop)
  - mika#1144  # original issue-scoped plan-find pattern
  - mika#1033  # >500-byte filter
milestone: 30  # Loop Trustworthiness
---

# Fix `_iterate_groom_loop` plan-find pattern — content fallback for date-prefix slug-tail filenames

## Problem

`_iterate_groom_loop` and two sibling functions (`_launch_revise_pilot`, `_write_canonical_callout`) discover the issue-scoped plan file via a brittle filename pattern at three callsites:

```bash
plan_path=$(find "$WORKTREE_DIR/docs/plans" -name "*-${ISSUE_NUM}-*-plan.md" -size +500c \
    2>/dev/null | sort -r | head -1)
```

The pattern requires the issue number to be embedded literally in the filename (e.g. `2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md` matches `*-1407-*-plan.md`).

The `/mika-groom-plan-only` skill instructs claude-pilot to save plans to `<repo>/docs/plans/<YYYY-MM-DD>-<NNN>-<type>-<slug-tail>-plan.md`. **Whether `<slug-tail>` includes the issue number is left to the pilot's interpretation**, and the pilot has been observed producing filenames that omit it.

### Founding incident

mika#771 dispatched dev-groom at 2026-06-06 17:22Z. The pilot session (`5e5490e4-56be-409a-b38c-8baeebb02c2c`, 22 turns, $1.89, 392s) wrote, committed, and pushed:

```
docs/plans/2026-06-06-003-feat-post-condition-guard-send-message-plan.md
```

The filename has **no issue number embedded**. `_iterate_groom_loop`'s `find -name "*-771-*-plan.md"` returned empty → `plan_path` empty → return 1 → architect never called → ticket lands in half-state: plan committed on branch, body callout never written.

This was n=2 of the architect-convergence class. n=1 was mika#1381's second chain test at 11:37Z — same exact error message, same exit path.

### Why content-fallback closes the class

The plan file itself contains the canonical ticket reference. Line 3 of mika#771's plan:

```markdown
**Ticket:** mika issue#771
```

The reference is present in the file's content even when absent from its filename. Grepping the recently-modified plan files for the `**Ticket:** mika [issue]#N` line (or YAML frontmatter `ticket: mika#N`) locates the correct plan without relying on filename convention drift.

Per [[feedback-prompt-enforcement-fragile]]: relying on the pilot's adherence to a filename convention is prompt-level enforcement — fragile. The plan's header is structurally written by the pilot per the same skill, but the failure mode (filename drift) is qualitatively different from the failure mode for content drift (the pilot writes a plan that doesn't reference its own ticket): the latter is a much louder, easier-to-catch failure than the former.

## Approach

Add a `_find_issue_plan` helper next to the existing iterate-loop primitives. The helper:

1. **Primary pass:** `find ... -name "*-${ISSUE_NUM}-*-plan.md" -size +500c` (current behavior).
2. **Fallback pass:** if primary returns empty, iterate over `find ... -name "*-plan.md" -size +500c` candidates and grep each for an anchored `**Ticket:** mika [issue]#N\b` or YAML `ticket: mika#N\b` line. Return the most-recent match.

Refactor the three brittle callsites (lines 1334, 1462, 1560) to call `_find_issue_plan` instead of inlining the `find` command.

### Why a helper rather than three duplicate fallback blocks

Single source of truth for plan discovery. If the pilot's filename or header convention evolves further, there's one place to update — not three. Sister rationale to `mika-arch-first-dogfood-2026-04-25` (verdict-parsing centralization).

### Anchor discipline on the content match

The fallback grep uses `^(\*\*Ticket:\*\*|ticket:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b`:

- **Line-anchored:** `^` rejects prose mentions of `#N` mid-paragraph
- **Keyword-anchored:** requires either `**Ticket:**` (markdown header) or `ticket:` (YAML frontmatter) — refuses casual mentions
- **Word-boundary:** `\b` after `${ISSUE_NUM}` rejects substring matches (e.g. `#77` matching against searches for `#7`)

The test suite covers a negative case: a plan that mentions `mika#1234` only in prose (without the anchored keyword) does NOT match.

### Most-recent-wins semantics

Both passes use `sort -r | head -1` (the existing convention). When both primary and fallback could match different files for the same issue (re-grooms with mixed filename conventions), **primary always wins** — the fallback only runs when primary returns empty. This preserves backward-compatibility with existing groomed plans that follow the filename convention.

## Acceptance Criteria

- [x] **AC1:** New `_find_issue_plan` helper exists in `skills/bundled/_shared/dispatch-lib.sh`, defined adjacent to other iterate-loop primitives (immediately before `_arch_ask`).
- [x] **AC2:** All three callsites (`_launch_revise_pilot`, `_write_canonical_callout`, `_iterate_groom_loop`) refactored to call `_find_issue_plan`. Each preserves its original WARN message and return semantics.
- [x] **AC3:** New test file `skills/bundled/_shared/tests/test_find_issue_plan.sh` with:
  - Primary-pass test (filename embeds issue number)
  - Fallback test for the mika#771 founding-incident filename shape
  - Fallback test for older `**Ticket:** mika#N` shape (no "issue" word)
  - Fallback test for YAML `ticket:` frontmatter
  - Negative test: prose `#N` mention without anchor keyword does NOT match
  - Sort-order: primary wins when both shapes exist; fallback picks most-recent when only fallback matches
  - Input guards: unset `ISSUE_NUM`, unset `WORKTREE_DIR`, missing `docs/plans` directory, sub-500-byte plan
- [x] **AC4:** All 11 new test assertions pass; existing `test_parse_disposition.sh` 56-assertion suite still passes.
- [x] **AC5:** Docblock comment in `_iterate_groom_loop` updated to reference the helper rather than the inline pattern.

## Phase 0 — Pin

Pinned against base SHA `e1ebebc3` (`fix(dispatch-lib): worktree-setup clobbers sub-repo .claude/commands (#1255 regression on every dispatch) (#1418)`, merged 2026-06-06).

Three callsites confirmed at the pinned SHA:

```
1334: plan_path=$(find "$WORKTREE_DIR/docs/plans" -name "*-${ISSUE_NUM}-*-plan.md" -size +500c \
1462: plan_path=$(find "$WORKTREE_DIR/docs/plans" -name "*-${ISSUE_NUM}-*-plan.md" -size +500c \
1560: plan_path=$(find "$WORKTREE_DIR/docs/plans" -name "*-${ISSUE_NUM}-*-plan.md" -size +500c \
```

(Functions: `_launch_revise_pilot`, `_write_canonical_callout`, `_iterate_groom_loop`.)

Test infrastructure pinned at `skills/bundled/_shared/tests/test_parse_disposition.sh` (the canonical test pattern this fix mirrors).

## Risks

1. **Content-pattern drift in future pilots.** If the pilot evolves to use a different ticket-reference shape (neither `**Ticket:** mika ...` nor `ticket: mika...`), the fallback would not match. Mitigation: the test file documents the three currently-known shapes; new shapes get added when observed (n=1 observe-don't-add).

2. **False-positive on plans with embedded sub-references.** A plan for mika#100 that references mika#1000 in an anchored line could in principle false-positive. Mitigation: `\b` word boundary after the issue number rejects substring matches. Test coverage includes a sort-order case proving most-recent semantics hold.

3. **Performance.** The fallback iterates over all `*-plan.md` files in `docs/plans/`. Today's repo has ~552 plan files; grepping ~552 small text files for one anchored regex line takes <100ms on the dev host. Acceptable for a per-dispatch operation.

## Verification

1. Run new test: `bash skills/bundled/_shared/tests/test_find_issue_plan.sh` — expect 11 passing, exit 0.
2. Regression: `bash skills/bundled/_shared/tests/test_parse_disposition.sh` — expect 56 passing, exit 0.
3. Bash syntax: `bash -n skills/bundled/_shared/dispatch-lib.sh` — expect "syntax ok".
4. Post-deploy live verification: re-dispatch mika#771 by adding the `ready` label; verify dev-groom completes, body callout writes, architect verdict appears in groom-verdict-trail.log. **This is the load-bearing test** — the fix is validated when the founding incident no longer reproduces.

## Sequencing

Implementation is a substrate-pivot (single file + tests + plan doc), shipping by-hand per the established #1414/#1415 morning-pattern. Branch: `fix/1421/iterate-groom-loop-plan-find-content-fallback`. PR open + deploy + re-dispatch all in one cycle.

The fix is freeze-window-safe because the substrate-pivot pattern unwedges the autonomous loop; CLAUDE.md prime directive: *"Autonomous loop always works."*
