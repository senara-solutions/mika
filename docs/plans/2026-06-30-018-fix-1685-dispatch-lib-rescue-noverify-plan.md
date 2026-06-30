---
issue: 1685
type: fix
date: 2026-06-30
---

# Plan — fix(dispatch-lib): post-flight rescue commits bypass pre-commit hook (mika#1685)

## Problem

`skills/bundled/_shared/dispatch-lib.sh` post-flight rescue paths call `git commit` WITHOUT `--no-verify`. The pre-commit hook (lefthook with `rust-clippy`) runs on every rescue commit. When the pilot leaves any clippy nit in the worktree, the hook rejects the commit → `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry` → parent task → blocked. The 95% complete work is stranded; no PR opens.

Hard evidence (2026-06-30 14:30-16:18Z): three blocked tasks (mika#1680, mika#1682, mika#1676) all carry the identical callback prefix `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry`. Modal cause across the 5-task wedge per Mika Prime bearing 2026-06-30 ~16:32Z (session `00000000`).

## Architectural lineage

- mika#1282 — original dirty-worktree rescue (line 890 `git commit`)
- mika#1396 — commit-pushed-no-pr rescue retry (line 924 `git commit` — cargo-fmt fallback)
- mika#1383 — auto-PR-create trailing content (line 1020 `git commit`)
- mika#1058 — callback can't retry pilot (the trap, downstream of this cause — separate fix)
- mika#1639 (CLOSED) — sibling permission-policy fix shipped earlier today

## Fix shape (one-line edits × 3 sites + rationale comment)

Add `--no-verify` to all three rescue `git commit` invocations in `skills/bundled/_shared/dispatch-lib.sh`:

- **Line 890** (mika#1282 dirty-worktree initial commit): `git -C "$WORKTREE_DIR" commit -m "..." --no-verify`
- **Line 924** (mika#1282 cargo-fmt retry commit): `git -C "$WORKTREE_DIR" commit -m "..." --no-verify`
- **Line 1020** (mika#1383 trailing-content commit): `git -C "$WORKTREE_DIR" commit -m "..." --no-verify`

**Rationale comment** added next to one of the calls (or in a top-of-block doc-comment):

```bash
# Rescue commits bypass the pre-commit hook (--no-verify) by design (mika#1685):
# the rescue path's purpose is to SALVAGE work for operator review, not to gate
# it on lint. A single clippy nit (one-line typo) should not strand 29-turn,
# $4-cost pilot work as a dead block. CI runs clippy as a check on the rescue
# draft PR, surfacing the same signal at the right layer — visible to the
# operator + the autonomous-loop's clippy-fix-retry path, not as a hard block.
```

## Implementation outline

1. **Read the three call sites** at `dispatch-lib.sh:890`, `:924`, `:1020`. Confirm each is the rescue-path commit (not a developer-tool commit elsewhere).

2. **Add `--no-verify` to each**. Position: after `-m "..."` argument, before any redirect (`2>&9`). Pattern: `git -C "$WORKTREE_DIR" commit -m "..." --no-verify`.

3. **Add rationale comment** above the line 890 block (the first/canonical site). Reference mika#1685 + the operator-review-not-lint-gate principle.

4. **Re-evaluate `cargo-fmt retry` mechanism (lines 920-930 area).** Architect-bearing question: does this retry still make sense once `--no-verify` ships? Two options:
   - **Keep it** — `cargo fmt` is still useful as best-effort formatting before the rescue commit. The retry-after-format is now a no-op because the first commit (also `--no-verify`) already succeeded.
   - **Remove it** — the retry's value (re-formatting after a fmt-hook failure) is moot since the hook no longer fires. Code path becomes dead.
   
   My read: keep it cheap (`cargo fmt` is fast, harmless on clean worktree); remove the second `git commit` retry since the first one always succeeds now. Architect can confirm.

5. **Regression test** — add a dispatch-lib integration test (or extend `skills/bundled/_shared/tests/test-dispatch-lib.sh`) that:
   - Seeds a worktree with a deliberate clippy warning (e.g., a `repeat().collect()`).
   - Runs the rescue path.
   - Asserts: rescue commit succeeds (returns 0), PR opens with `wip-rescue` label, no `PIPELINE FAILURE` emitted.

## Acceptance criteria

- **AC1** — `dispatch-lib.sh:890,924,1020` rescue `git commit` invocations include `--no-verify`. Verified by reading the PR diff.

- **AC2** — Rationale comment added above the line 890 block (or equivalent canonical site), referencing mika#1685 + the operator-review-not-lint-gate principle. Comment includes the substring "CI runs clippy as a check on the rescue draft PR" or equivalent semantic.

- **AC3** — Regression test exists: a worktree with a deliberate clippy nit successfully rescues via the dispatch-lib path. PR opens, no `PIPELINE FAILURE`. Test placement in `skills/bundled/_shared/tests/test-dispatch-lib.sh` if it exists; else implementer documents manual verification in PR body.

- **AC4** — `cargo-fmt retry` mechanism architect-bearing decision applied: either kept-and-noted (retry is now defensive) or removed (dead code post-fix). Either acceptable; PR body documents the choice.

- **AC5** — Regression check: a worktree with NO clippy issues still rescues cleanly. The `--no-verify` doesn't break the happy path (it just skips the hook that would've also passed). Verified by existing rescue smoke tests OR new test.

## Out of scope

- **mika#1058 callback-can't-retry-pilot trap.** Separate substrate ticket (Concern 1 per Prime bearing — the trap, downstream of this cause). After this lands, Concern 1's urgency drops sharply because failures land as draft PRs instead of blocks.
- **Permission-policy errs-strict class (Concern 3).** Separate substrate ticket (mika#1686). Affects pilots BEFORE clippy — different cause class.
- **Silent pilot death (Concern 4).** Observation-class ticket (mika#1687). Different mechanism.
- **The 6 wedged tickets' clippy fixes.** Per Prime: recovery via rescue-to-draft (not hand-fix) AFTER this lands. Operator action.

## Operational composition

Per Prime ruling 2026-06-30 ~16:32Z: **DO NOT re-toggle ready on the 6 wedged tickets until this fix lands in the running substrate.** Re-toggling pre-fix burns $4×6 against the same hook reject. Recovery for the 6 wedged tickets is rescue-to-draft (preserve their existing branch work — the 29-turn / $4-cost pilots' implementation is real and shouldn't be re-burned), gated on this fix landing first.

## Files involved

- `skills/bundled/_shared/dispatch-lib.sh:890,924,1020` — three rescue `git commit` calls
- `skills/bundled/_shared/tests/test-dispatch-lib.sh` (if exists) — regression test
- No Rust/source-code changes; no schema migration

## Verification

- **Static:** PR diff shows `--no-verify` added at exactly the three rescue commit sites, no other commits. Rationale comment present.
- **Synthetic (AC3):** dispatch-lib test harness rescue with deliberate clippy seed — passes.
- **Regression (AC5):** dispatch-lib test harness rescue with no issues — still passes.
- **Live (post-merge):** the next pilot session that exits with a clippy nit successfully rescues to a draft PR + `wip-rescue` label. Verified in the autonomous-loop's next dispatch.

## References

- mika#1282 — original dirty-worktree rescue
- mika#1396 — commit-pushed-no-pr rescue retry
- mika#1383 — auto-PR-create
- mika#1058 — callback deferred dispatch (the trap, separate)
- mika#1058 callback turn constraint — why the retry is trapped
- Mika Prime bearing 2026-06-30 ~16:32Z (session `00000000`) — modal cause ratification + ordering ruling
- Hard evidence tasks: `dd81ff3b`, `48d03390`, `12f621bc` (all `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook`)
- `skills/bundled/_shared/dispatch-lib.sh:890,924,1020` — edit targets
