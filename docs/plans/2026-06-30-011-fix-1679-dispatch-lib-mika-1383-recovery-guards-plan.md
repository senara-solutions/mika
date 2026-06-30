---
issue: 1679
type: fix
date: 2026-06-30
---

# Plan — fix(dispatch-lib): mika#1383 auto-PR-create path bypasses mika#1613 recovery-pending guards (mika#1679)

> **rev 2 (2026-07-01) — design overturned to Resolution 2 (defer-to-Path-B).** The original rev-1 plan added four coordinated edits to the mika#1383 auto-PR-create block ("Path A") to make it emit `--draft` + `RECOVERY_PENDING` + a `wip()` marker commit + a rescue-body header. Implementation-time confidence check revealed that a second, **already-correct** rescue path ("Path B", mika#1396 `commit-pushed-no-pr`) exists and is **shadowed** by Path A. mika-arch ESCALATED (session `2d397bee`); Mika Prime ruled R2 on bearing grounds (single-source-of-truth on a churning safety contract). See **Revision history** at the bottom for the full lineage. The rev-1 four-edit shape is preserved in Revision history for traceability.

## Problem

mika#1613's fix (PR #1677, merged + deployed 2026-06-30 13:48Z) added two qa-webhook guards in `skills/bundled/self-dev-webhook-qa/system_prompt.md` AND qa-review's Step 1.5 rescue-class detection in `skills/bundled/qa-review/system_prompt.md`:
- **Guard 1** — checks `unpushed_recovery_pending: true` task metadata flag (set when callback parses `RECOVERY_PENDING: true` from RESULT).
- **Guard 2** — `isDraft AND ^wip\(` conjunction check (defense-in-depth; parses `gh pr view --json commits --jq '.commits[-1].messageHeadline'`).
- **qa-review Step 1.5** — looks for `## Auto-rescued PR (dispatch-lib recovery, class: ...)` header in PR body to classify as rescue and emit `hold[review]`.

**The observable bug:** the mika#1383 auto-PR-create path opens a **non-draft** PR with a plain body and no marker, so all three layers bypass and the rescue PR is eligible for autonomous un-draft + auto-merge without operator review. Hard evidence: mika#PR1678 (mika#1645 impl, opened 2026-06-30 14:05Z post-deploy, `isDraft: false`, no marker) and mika#PR1683 (mika-qa auto-approved at 15:54:35Z because the body lacked the rescue header).

**What the rev-1 diagnosis missed — the shadowing (rev-2 discovery, hard evidence @ `dispatch-lib.sh` HEAD `ad001aaf`):** there are now **two** PR-creation paths in dispatch-lib with the *same* trigger condition, and the first shadows the second:

- **Path A** — the mika#1383 "structural completion gate", `skills/bundled/_shared/dispatch-lib.sh:1023-1114`, inside `_post_flight_recovery()` (def@757). Guard at 1040: `SKILL=dev-pilot && POST_RUN_HEAD set && PRE_RUN_HEAD != POST_RUN_HEAD && WORKTREE_DIR && BRANCH`. It opens a **non-draft** PR (1091-1101), then at 1198-1204 re-queries and **sets the global `PR_URL`** (documented global at 766-768, no `local`) and emits a `PR:` line.

- **Path B** — the mika#1396 `commit-pushed-no-pr` rescue, `dispatch-lib.sh:2488-2567`, inside `dispatch_claude_pilot()` (def@2312), runs after `_run_claude_pilot` (2421) → `_push_branch` (2486). Guard at 2502: `[ -z "$PR_URL" ] && PRE_RUN_HEAD set && POST_RUN_HEAD set && PRE != POST && SKILL=dev-pilot`. This path **already** does everything rev-1 wanted to add to Path A: `--draft` (2523), the `## Auto-rescued PR (dispatch-lib recovery, class: commit-pushed-no-pr)` header + `<!-- rescue-pipeline-verified: no -->` marker (2526-2528), `RECOVERY_PENDING: true` (2558), the `wip-rescue` label (2547), the canonical `PR:` line (2557), and a purpose-built title via `_derive_recovery_pr_title("commit-pushed-no-pr", …)` (2207-2214).

Because Path A runs first and sets the global `PR_URL`, Path B's `[ -z "$PR_URL" ]` guard is false → **Path B never fires for the commit-pushed-no-pr case**. Path B's `commit-pushed-no-pr` branch is effectively dead code (it survives only as a fallback when Path A's own `gh pr create` errors). The rev-1 four-edit plan would have made Path A *duplicate* Path B's correct behavior — two implementations of one safety contract that must stay in sync forever, on a contract that demonstrably churned four times this week (draft, marker, header, label).

## Architectural lineage

- mika#1613 (CLOSED via PR #1677) — parent fix, incomplete. Structurally correct for the mika#1282 path; never covered the mika#1383 path.
- mika#1282 — original dirty-worktree rescue (where the marker + `--draft` + rescue-body convention started). Flows through Path B's `dirty-worktree` class.
- mika#1396 — `commit-pushed-no-pr` rescue (Path B). **This is the already-correct handler the rev-1 plan did not reference and Path A shadows.**
- mika#1383 — auto-PR-create structural-completion gate (Path A). Predates / duplicates mika#1396's coverage and pre-empts it via the global `PR_URL`.
- mika#1618 — qa-review Step 1.5 rescue-class detection (the body-header consumer).
- mika#1352 — canonical `PR:` line convention.

## Fix shape (Resolution 2 — defer to Path B)

**Single structural edit + one defense-in-depth decision, both in `skills/bundled/_shared/dispatch-lib.sh`.** The principle (Mika Prime bearing): a churning safety contract gets a single source of truth. Path B already owns the correct `commit-pushed-no-pr` rescue-PR shape; the fix is to stop Path A from shadowing it, not to duplicate it.

### Edit 1 (core) — delete Path A's Phase-B PR-creation; let Path B own it

Remove the **Phase B** block of the mika#1383 gate — the PR-existence check + `gh pr create` + result append (`dispatch-lib.sh:1064-1113`, the `EXISTING_PR=""` discovery through the end of the `if [ -z "$EXISTING_PR" ]` block, inclusive of its failure branch). **Keep Path A's Phase A** (trailing-dirty-content rescue, 1044-1062) untouched — it commits trailing dirty content with a `wip()` prefix and advances `POST_RUN_HEAD` so Path B sees the latest commits; it is complementary to Path B, not duplicate.

After Edit 1, the mika#1383 trigger flows: `_post_flight_recovery` runs Phase A only → does **not** set `PR_URL` (the 1198-1204 re-query returns empty when no PR exists on the branch) → control returns to `dispatch_claude_pilot` → Path B's guard sees `[ -z "$PR_URL" ]` true → Path B fires and opens the correct draft rescue PR.

The Phase-A wip-prefix update of `POST_RUN_HEAD` keeps Path B's `PRE != POST` guard satisfied. The "PR already exists" case stays safe: if a PR exists, the 1200 re-query sets `PR_URL` → Path B's `[ -z "$PR_URL" ]` is false → Path B no-ops → no double-create.

**Reachability proof (Mika Prime's required pre-condition — "A shadows B" ≈ but ≠ "B runs when A is gone"):** verified at HEAD `ad001aaf`. Between `_post_flight_recovery` (called inside `_run_claude_pilot` @741) and Path B's guard (@2502), the only early-return is `_check_pilot_force_push` (@2427). That function returns 0 unconditionally for dev-pilot (`dispatch-lib.sh:1265` — `[ "$SKILL" = "dev-groom" ] || return 0`), so it never short-circuits the dev-pilot path. The dev-groom block (2450-2484) is skipped for `SKILL=dev-pilot`. `_push_branch` (2486) does not touch `PR_URL`. Therefore, with Path A's PR-creation removed, Path B is reachable and fires for the mika#1383 trigger with `PR_URL` empty. **No fail-silent-no-PR risk for dev-pilot.**

### Edit 2 (defense-in-depth — architect's call on second-pass) — preserve Guard 2 coverage for commit-pushed-no-pr

Guard 2 (`isDraft AND ^wip\(`) parses the PR's **head-commit headline**. Path B opens the PR from the pilot's existing commits — whose head-commit headline is the pilot's own conventional-commit subject (`fix(...)`, `feat(...)`), **not** `wip(`. So under R2, the `commit-pushed-no-pr` rescue fires **Guard 1** (`RECOVERY_PENDING` → metadata flag) and **qa-review Step 1.5** (rescue-body header) but **not** Guard 2. This is unchanged from Path B's pre-existing behavior — Guard 2 only ever fired for the `dirty-worktree` class, where the mika#1282 rescue commit *is* `wip()`-prefixed.

The rev-1 plan added a `wip()` marker commit (its "Edit 3") specifically to make Guard 2 fire. **Recommended sub-option:** relocate that single defense-in-depth commit into **Path B's `commit-pushed-no-pr` branch** (an empty `git commit --allow-empty -m "wip(mika#1383): auto-PR-create rescue for ${REPO}#${ISSUE_NUM}…"` + `git push` immediately before Path B's `gh pr create`, guarded to the `commit-pushed-no-pr` class only). This restores all three guard layers on the single-source-of-truth path, matching rev-1's defense-in-depth intent without re-introducing duplication. **Architect decides on second-pass:** (a) include Edit 2 (all three guards fire, +1 empty commit in rescue PR history), or (b) omit it and rely on Guard 1 + Step 1.5 (two independent guards; Guard 2 stays dirty-worktree-only as today). Recommendation: **(a)** — the contract churned four times this week; belt-and-suspenders on a safety path is cheap.

## Implementation outline

1. **Locate Path A's Phase B** in `dispatch-lib.sh` (the `EXISTING_PR=""` discovery at ~1064 through the close of the `if [ -z "$EXISTING_PR" ]` block at ~1113, including the `gh pr create` and both success/failure RESULT appends). Confirm the boundary: Phase A (trailing-dirty rescue, 1044-1062) ends just before `EXISTING_PR=""`; the function continues at the `# Post-flight plan validation` block (~1117) after the deleted region.

2. **Delete Phase B.** Remove the PR-existence check, the `gh pr create` invocation, and the result-append lines. Leave the surrounding `if [ "$SKILL" = "dev-pilot" ] && … ]; then` structure and Phase A intact. If removing Phase B leaves the outer `if` containing only Phase A, keep the `if` (Phase A still needs its guard).

3. **(Edit 2, if architect approves)** Add the `commit-pushed-no-pr`-scoped empty `wip()` marker commit + push to Path B, immediately before the `gh pr create` at 2519, inside the `if [ "$RECOVERY_CLASS" = "commit-pushed-no-pr" ]`-equivalent branch (guard so the `dirty-worktree` class — which is already `wip()`-prefixed — does not get a second empty commit).

4. **Verify Path B's `dirty-worktree` regression** — read 2499-2567 to confirm the `dirty-worktree` class still emits its header, `--draft`, `RECOVERY_PENDING: true`, `wip-rescue` label, and `PR:` line. Edit 1 must not touch this block.

5. **Test surface** — extend `skills/bundled/_shared/test-dispatch-lib.sh` (Test 15 already covers the mika#1383 gate) and/or `_shared/tests/`. Assert: (a) the mika#1383 commit-pushed-no-pr trigger no longer creates a non-draft PR from Path A (the `gh pr create` string is gone from the Path A block), (b) the trigger routes to Path B (the rescue body header + `RECOVERY_PENDING: true` + `--draft` are the only PR-create path for this trigger), (c) the `dirty-worktree` class is unchanged. Implementer first task: grep the gate block + Test 15 to anchor the assertions.

## Acceptance criteria

- **AC1** — Path A's Phase-B PR-creation is removed from the mika#1383 gate in `dispatch-lib.sh`: no `gh pr create` invocation remains inside `_post_flight_recovery()`. Verified by reading the PR diff (the `EXISTING_PR` discovery + `gh pr create` block at the old ~1064-1113 is deleted).

- **AC2** — Path A's **Phase A** (trailing-dirty-content rescue, old 1044-1062) is **retained** unchanged — it still commits trailing dirty content with `wip()` and advances `POST_RUN_HEAD`. Verified by reading the PR diff (Phase A block untouched).

- **AC3** — For the `commit-pushed-no-pr` trigger (pilot committed + pushed, no PR, `SKILL=dev-pilot`), the resulting rescue PR is produced by **Path B** and: (a) opens as **draft** (`gh pr view --json isDraft` → `true`), (b) RESULT contains `RECOVERY_PENDING: true`, (c) PR body contains both `## Auto-rescued PR (dispatch-lib recovery, class: commit-pushed-no-pr)` and `<!-- rescue-pipeline-verified: no -->`, (d) the PR carries the `wip-rescue` label, (e) RESULT contains a canonical `PR:` line. Validated by the test harness and/or live evidence in the PR body.

- **AC4 (reachability)** — Evidence in the PR body (test or trace) that with Path A's PR-creation removed, Path B's `[ -z "$PR_URL" ]` guard fires for the mika#1383 trigger — i.e. the trigger produces exactly **one** rescue PR (no fail-silent-no-PR, no double-create). The reachability proof in §Fix shape Edit 1 is transcribed or cited in the PR description.

- **AC5 (regression)** — The mika#1282 `dirty-worktree` rescue class (Path B, `RESCUED_DIRTY_WORKTREE=1`) is unchanged: still emits the rescue header, `--draft`, `RECOVERY_PENDING: true`, `wip-rescue` label, and `PR:` line. Verified by reading the Path B block in the PR diff — no inadvertent edits.

- **AC6 (Guard 2 — conditional on Edit 2)** — *If the architect approves Edit 2:* after a `commit-pushed-no-pr` rescue, `gh pr view <N> --json commits --jq '.commits[-1].messageHeadline'` returns a string starting with `wip(mika#1383):`, so Guard 2's `isDraft AND ^wip\(` conjunction fires. *If the architect omits Edit 2:* this AC is dropped and the PR description documents that Guard 1 + qa-review Step 1.5 are the two active guards for this class (Guard 2 remains dirty-worktree-only, unchanged from current behavior).

- **AC7 (integration test)** — `test-dispatch-lib.sh` (or `_shared/tests/`) covers: (a) the mika#1383 commit-pushed-no-pr trigger no longer creates a PR from Path A, (b) it routes to Path B producing draft + marker + header + label + `PR:` line, (c) the `dirty-worktree` class still works. PR body includes the test invocation + pass output. If the harness can't drive a full rescue end-to-end, AC7 reduces to documented manual/trace verification in the PR body (architect-ratified path, as in rev-1 AC4).

## Out of scope

- **Engine-side guard for non-marker, non-draft PRs.** Would require qa-webhook architectural redesign. File a separate ticket if R2's coverage proves insufficient post-deploy.
- **Migration of existing non-draft wip-rescue PRs.** PR #1678 + any in-flight non-draft rescues from before this fix remain operator-managed. Fix prevents future occurrences.
- **Broader unification of the mika#1282/mika#1396/mika#1383 rescue surface beyond removing the Path A shadow.** R2 removes the one shadowing duplication; any further consolidation (e.g., folding Phase A into Path B) is a separate refactor, filed only if more drift surfaces.

## Files involved

- `skills/bundled/_shared/dispatch-lib.sh` — Edit 1 (delete Path A Phase B, ~1064-1113) + Edit 2 if approved (wip-marker commit in Path B's `commit-pushed-no-pr` branch, ~2519)
- `skills/bundled/_shared/test-dispatch-lib.sh` and/or `skills/bundled/_shared/tests/` — AC7 integration test (extend Test 15)
- No Rust/source-code changes; no schema migration; no engine guard changes

## Verification

- **Static:** read the PR diff. Path A's `gh pr create` block is gone; Phase A retained; Path B's `dirty-worktree` block untouched; (if Edit 2) the `commit-pushed-no-pr` wip-marker commit added under its class guard.
- **Synthetic test (AC7):** drive the mika#1383 trigger (pilot commits + pushes, no PR) against the harness; assert the PR is created by Path B as draft with the rescue header + `RECOVERY_PENDING: true`, and the `dirty-worktree` path still works.
- **Live verification post-merge:** the next pilot session that hits the commit-pushed-no-pr path opens a single draft PR with the rescue header + marker; `gh pr view <N> --json isDraft` → `true`; `SELECT json_extract(metadata, '$.unpushed_recovery_pending') FROM tasks WHERE id = <task>` → `true`.

## References

- mika#1613 (CLOSED) — parent fix (PR #1677, merged 13:48Z)
- mika#1282 — original dirty-worktree rescue (Path B `dirty-worktree` class)
- mika#1396 — `commit-pushed-no-pr` rescue (Path B) — the already-correct handler Path A shadowed
- mika#1383 — auto-PR-create structural completion gate (Path A) — the shadowing block
- mika#PR1678 (MERGED) — bypassed case (non-draft via Path A 14:05Z post-deploy)
- mika#PR1683 — layer-3 bypass evidence (mika-qa auto-approved 15:54:35Z, no rescue header)
- mika#1684 — Edit-4-source in rev-1 (rescue body header); subsumed: Path B already emits the header, so #1684's concern is resolved by routing to Path B. Close as subsumed when this lands.
- mika#1618 — qa-review Step 1.5 rescue-class detection
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — Guard 1 + Guard 2 source
- `skills/bundled/qa-review/system_prompt.md` — Step 1.5 rescue-header detection
- `skills/bundled/_shared/dispatch-lib.sh:1023-1114` — Path A (mika#1383 gate; Phase A keep / Phase B delete)
- `skills/bundled/_shared/dispatch-lib.sh:2488-2567` — Path B (mika#1396 rescue; the single source of truth R2 defers to)
- `skills/bundled/_shared/dispatch-lib.sh:1265` — `_check_pilot_force_push` dev-pilot no-op (reachability proof)
- mika-arch ESCALATE, session `2d397bee` (2026-07-01) — confirmed the shadowing + that its own "refactor only if further drift surfaces" criterion is met; ruled operator-ratify.
- Mika Prime bearing, session `00000000-…` (2026-07-01) — ruled **R2**, bearing-scope (not milestone): "R1 creates two implementations of one safety contract that must stay in sync forever; R2 leaves one correct implementation. On a rescue path whose contract demonstrably churns, single-source-of-truth is the correctness floor." Required the reachability pre-condition check (now satisfied, §Fix shape Edit 1).

## Revision history

- **rev 1 (2026-06-30):** original groomed plan (mika-arch first-pass ITERATE F1 BLOCKING → revisions → second-pass GROOMED). Fix shape: four coordinated edits to Path A (the mika#1383 gate) — Edit 1 emit `RECOVERY_PENDING`, Edit 2 add `--draft`, Edit 3 add a `wip()` marker commit (to fire Guard 2), Edit 4 emit the rescue-body header (absorbed from mika#1684). Explicitly scoped *out* "unifying mika#1282 + mika#1383 rescue blocks" with the criterion "refactor only if further drift surfaces." **Superseded by rev 2** — see below.
- **rev 2 (2026-07-01):** **design overturned to Resolution 2 (defer-to-Path-B).** Implementation-time confidence check (the rev-1 plan referenced #1282/#1383 but not #1396) revealed Path B (mika#1396 `commit-pushed-no-pr`) already implements the full correct rescue-PR shape and is shadowed by Path A's earlier global `PR_URL` set. mika-arch (session `2d397bee`) confirmed the shadowing, confirmed its own drift-criterion is now met (this finding *is* the further drift), and **ESCALATED** for operator ratification. Mika Prime (session `00000000-…`) ruled **R2** on bearing grounds (bearing-scope, not milestone): single-source-of-truth on a churning safety contract; R1 would institutionalize a two-implementations-one-contract divergence trap. Prime required, before calling R2 safe, an explicit reachability check that Path B fires once Path A's PR-creation is gone — **satisfied** (§Fix shape Edit 1 proof; `_check_pilot_force_push` is a dev-pilot no-op). Rev-2 fix shape: **delete Path A's Phase-B PR-creation** (Edit 1), **keep Path A Phase A**, defer PR creation to Path B; **optionally relocate rev-1's Edit 3 wip-marker commit into Path B** (Edit 2, architect's call) to preserve Guard 2's defense-in-depth. ACs remapped from "verify Path A output" to "verify Path B output for this trigger" + a reachability AC. Could-not-address: none — all rev-1 findings (draft, marker, header, Guard 2) are satisfied by Path B's existing behavior plus optional Edit 2; the four-edit *mechanism* is overturned, but every guard *outcome* rev-1 sought is preserved.
