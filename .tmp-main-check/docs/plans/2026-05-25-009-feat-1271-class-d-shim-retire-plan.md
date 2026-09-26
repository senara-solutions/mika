# Retire Class D body-callout recovery shim (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` / Class D `(i) Retire` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`.
**Sub-PR sequence:** **Sub-PR 7b of mika#1271 — Class D shim retirement.**
  - PRs #1273 (`f2bef21`), #1274 (`1eb5a03`), #1275 (`30917d7`), #1276 (`96fd2281`), #1277 (`9c20096e`), #1278 (`f58b5450`), #1279 (`288375ee`).
  - **This PR (7b)**: delete `_verify_and_write_body_callout` function + its post-flight call site in `_run_claude_pilot`. The canonical writer (sub-PR 6) is the sole structural authority for the body callout.
  - **Sub-PR 8 (follow-up)**: update dev-groom skill prompt to drop the pilot's redundant architect invocation + redundant organic body-callout write. That closes the cost regression introduced when 7a flipped the iterate loop default-on.

## Live-exercise evidence backing this retirement

Sub-PR 7a's first-observation test on mika#1267 (2026-05-25 18:53–18:59 UTC after `make deploy` installed the substrate) confirmed:

- The canonical writer fires correctly on GROOMED — prepended a `first-pass (READY) → second-pass (GROOMED) — session-id: 2e8f1d22-21a6-4da6-8878-cb246ba7651b` block above the existing body content.
- Dispatch gate satisfied: the canonical block carries the literal `second-pass (GROOMED)` marker that `executor.rs::check_grooming_markers` greps for.
- Clean prepend with blank-line separation; no interference with existing body content.

Class D's recovery role is now redundant — every drift case Class D used to catch (plan committed but no callout written) is now resolved structurally by the iterate loop's architect convergence + canonical write.

## What changes

### Deleted from `dispatch-lib.sh`

1. **Function `_verify_and_write_body_callout`** (lines 162-311 in the pre-edit file, ~150 lines including dual-write doc + cat-file-guard + uncommitted/local-only/committed-and-pushed fallback branches + recovery commit logic + recovery push logic + gh issue edit).
2. **Post-flight call site** in `_run_claude_pilot` (lines 609-615): the 7-line `if [ "$SKILL" = "dev-groom" ] && ... ; then _verify_and_write_body_callout "$REPO" "$ISSUE_NUM" "$WORKTREE_DIR" "$BRANCH"; fi` block.

### Updated comments in `dispatch-lib.sh`

- `_push_branch`: updated the "first-push case (no origin/$BRANCH ref)" comment to remove the Class D push reference (Class D's recovery push is gone).
- `_write_canonical_callout` docstring: replaced "distinct from `_verify_and_write_body_callout` (mika#1123) which writes a RECOVERY callout" with "sole structural writer as of sub-PR 7b".
- `_iterate_groom_loop` docstring: replaced "downstream Class D recovery (mika#1123) until sub-PR 7b retires the shim" with "the pilot's organic write in the dev-groom skill prompt remains as fallback until the dev-groom-prompt-update follow-up ships".
- `_iterate_groom_loop`'s plan-file find comment: replaced "Same pattern as `_verify_and_write_body_callout`" with the direct citation of the pattern shape.
- `dispatch_claude_pilot` wiring-point comment: rewrote to reflect that the iterate loop + canonical writer is the sole structural authority for the body callout; pilot's organic write remains as fallback.
- Echo message on iterate-loop non-converge: replaced "Class D recovery (mika#1123) downstream catches drift" with "pilot's organic write remains as fallback (dev-groom skill prompt)".

### Test file changes

1. **Removed Test 8** (mika#1144 — "no unscoped fallback find call"): the function is gone, the regression class it guarded against is no longer reachable.
2. **Removed Test 11** (mika#1204 — Class D cat-file guard, recovery commit shape, fallback callouts, push retry): same — function deleted, all 12 assertions inside are obsolete.
3. **Updated the 7a "Class D still invoked" assertion** at line 501: was `assert_contains "_run_claude_pilot still invokes Class D recovery shim (defense-in-depth)" "_verify_and_write_body_callout"`. Now inverted to `assert_not_contains "_run_claude_pilot no longer invokes Class D recovery shim"` and supplemented with a function-definition-absence check via `declare -f`.
4. **Updated docstring comments** in two places (line 660, line 717) to reference the retirement.

Net delta: -17 assertions (Test 8 + Test 11 + 1 7a assertion replaced with 2 new ones).

## Acceptance criteria

- [ ] **AC1:** `_verify_and_write_body_callout` is absent from `skills/bundled/_shared/dispatch-lib.sh` (grep returns zero matches for the function name within function-definition context).
- [ ] **AC2:** The post-flight call site in `_run_claude_pilot` is removed — no `_verify_and_write_body_callout "$REPO" "$ISSUE_NUM" ...` invocation exists.
- [ ] **AC3:** `_write_canonical_callout` remains the sole structural writer of body callouts (call count in `_iterate_groom_loop` = 2, unchanged from sub-PR 6).
- [ ] **AC4:** `_iterate_groom_loop` and `_escalate_groom` call shapes unchanged: 3 `_escalate_groom` call sites, 2 `_cleanup_iterate_findings` calls (GROOMED-only preservation invariant).
- [ ] **AC5:** Comment hygiene: no `Class D recovery (mika#1123) downstream catches drift` strings remain in code paths; all surviving Class D references are intentional historical context in docstrings (sub-PR 7b retirement narrative).
- [ ] **AC6:** `bash -n` exit 0 on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC7:** Test suite: pre-existing failure count (6) unchanged. Net assertion count decreased by 17 (Tests 8 + 11 deleted, defense-in-depth assertion replaced).

## Behavioral contract after 7b

1. `_run_claude_pilot` runs the dev-groom pilot (still using the OLD pilot-owns-architect skill prompt — sub-PR 8 changes that).
2. Pilot may write its own organic body callout (still happens — dev-groom skill prompt is unchanged in 7b).
3. `_iterate_groom_loop` runs unconditionally. Architect first/second pass. On GROOMED → `_write_canonical_callout` prepends the canonical block.
4. `_push_branch` + `_deliver_callback` finalize.

**Operator-visible state on GROOMED:** body carries 2 callout blocks (canonical from dispatch-lib + pilot's organic). Down from 3 blocks under 7a (canonical + Class D recovery + pilot's organic). The Class D recovery middle layer is gone.

**Drift case (where Class D used to fire):** if the iterate loop doesn't converge AND the pilot's organic write also drifts (no callout written), there is no fallback. The dispatch gate fails on a subsequent dispatch — same as the pre-mika#1123 production posture. The architect's "(i) Retire" verdict explicitly authorized this trade-off: the iterate loop replaces detection-and-fail-back with detect-the-real-failure-and-fix-it-loudly. mika#1033 is the precedent — the ESCALATE flow's structured PIPELINE FAILURE markers are the operator-visible signal.

## What does NOT ship in this sub-PR (8 scope)

- **dev-groom skill prompt update** — the pilot still invokes its own architect (redundant with dispatch-lib's iterate loop) AND writes its own organic body callout (redundant with the canonical writer). The cost regression (architect-call doubling per groom) is unresolved until sub-PR 8 lands. **7b's cost regression resolution is incomplete without the prompt update.** That dev-groom-prompt-update is filed and shipped separately because it is content-contract scope, not structural plumbing.
- **mika#1272** (paraphrased dispositions) — separate ticket; the iterate loop's `_parse_disposition` already tolerates the canonical forms but the paraphrased-tolerant variant ships independently.

## Test plan

127 structural assertions pass (down 17 from sub-PR 7a's 144 — Tests 8 + 11 about retired Class D invariants deleted). 6 pre-existing failures unchanged.

Live runtime exercise: sub-PR 7a already proved the canonical writer fires on a real groom dispatch. 7b's only behavioral change is the deletion of a redundant recovery path. The canonical writer continues to work exactly as observed on mika#1267 — and the next operator-driven (or autonomous) groom under 7b will produce a body callout via the canonical writer alone, no Class D recovery in the middle.

## Provenance

- mika#1271 parent ticket, milestone#26.
- Sequence: PRs #1273 → #1274 → #1275 → #1276 → #1277 → #1278 → #1279 → **this**.
- Architect contract: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` — Class D verdict `(i) Retire`.
- mika#1123 — Class D recovery shim (now retired).
- mika#1033 — detect-and-fail-loudly precedent for ESCALATE; the structural surface replacing Class D's silent-recovery role.
- Friend-peer sharpenings retained: preserve-on-ESCALATE invariant, session-id symmetry, `.iterate/` ownership, idempotency check shape — all unchanged in 7b.
- Live-exercise evidence: mika#1267 dispatch at 2026-05-25 18:53–18:59 UTC, session `2e8f1d22-21a6-4da6-8878-cb246ba7651b`, canonical writer fired correctly with `first-pass (READY) → second-pass (GROOMED)` shape on the live body.
