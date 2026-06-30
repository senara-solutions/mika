---
issue: 1679
type: fix
date: 2026-06-30
---

# Plan — fix(dispatch-lib): mika#1383 auto-PR-create path bypasses mika#1613 recovery-pending guards (mika#1679)

## Problem

mika#1613's fix (PR #1677, merged + deployed 2026-06-30 13:48Z) added two qa-webhook guards in `skills/bundled/self-dev-webhook-qa/system_prompt.md` AND qa-review's Step 1.5 rescue-class detection in `skills/bundled/qa-review/system_prompt.md`:
- **Guard 1** — checks `unpushed_recovery_pending: true` task metadata flag (set when callback parses `RECOVERY_PENDING: true` from RESULT).
- **Guard 2** — `isDraft AND ^wip\(` conjunction check.
- **qa-review Step 1.5** — looks for `## Auto-rescued PR (dispatch-lib recovery, class: ...)` header in PR body to classify as rescue and emit `hold[review]`.

**All three layers bypass for mika#1383's auto-PR-create path:**

1. **mika#1383 doesn't emit `RECOVERY_PENDING: true` marker.** The marker is only emitted in the mika#1282 dirty-worktree rescue block at `skills/bundled/_shared/dispatch-lib.sh:2524-2526`. The mika#1383 auto-PR-create tail at `dispatch-lib.sh:1067` only appends a descriptive `dispatch-lib (mika#1383): auto-created PR ...` line, no marker. → Guard 1 doesn't fire.

2. **mika#1383 opens PRs as non-draft.** The `gh pr create` invocation at `dispatch-lib.sh:1054-1059` lacks `--draft`. → `isDraft = false` → Guard 2's conjunction (`isDraft = true AND ^wip\(`) fails. PR opens normal, eligible for auto-merge.

3. **mika#1383 doesn't write `## Auto-rescued PR` header in PR body.** The body it writes is `"Auto-created by dispatch-lib (mika#1383 structural completion gate)..."` — no rescue header. → qa-review Step 1.5 short-circuits with "this is not a rescue PR — skip to Step 2 normally" → proceeds to standard review → approves. **Hard evidence: mika#PR1683 mika-qa approval at 2026-06-30 15:54:35Z despite draft + wip-rescue label** (manual draft conversion held the merge gate; otherwise this would have auto-merged). Filed as mika#1684; absorbed here as Edit 4 per Mika Prime bearing 2026-06-30 ~16:02Z (architect-coherent producer-side fix in one PR).

**Hard evidence:** mika#PR1678 (mika#1645 implementation, opened 2026-06-30 14:05Z post-mika#1613-deploy). The commit message `wip(mika#1645): impl staged by post-flight recovery (mika#1282)` confirms a rescue fired, but `isDraft: false` indicates the PR was opened by the mika#1383 path (not mika#1282 which sets `--draft`). Neither Guard 1 nor Guard 2 fired — the auto-merge vulnerability remains.

## Architectural lineage

- mika#1613 (CLOSED via PR #1677) — parent fix, incomplete. The fix shape was structurally correct for the mika#1282 path; this fix completes coverage for mika#1383.
- mika#1282 — original dirty-worktree rescue (where the marker + `--draft` convention started).
- mika#1383 — auto-PR-create structural-completion gate (where the marker + `--draft` were never added).
- mika#1352 — canonical `PR:` line convention (the precedent for line-shape standardization in dispatch-lib RESULT).

## Fix shape

Three coordinated edits to a single file (`skills/bundled/_shared/dispatch-lib.sh`), architect-F1-narrowed:

### Edit 1 — emit `RECOVERY_PENDING: true` marker in mika#1383 path

After `dispatch-lib.sh:1067` (the line appending `dispatch-lib (mika#1383): auto-created PR ${EXISTING_PR} ...` to RESULT), also append `RECOVERY_PENDING: true` to RESULT — mirroring the mika#1282 path's structure at lines 2524-2526.

Operator-review semantics: a mika#1383 auto-PR-create means the pilot session committed and pushed but ran out of turns before invoking `gh pr create`. dispatch-lib opened the PR on its behalf — same "operator must review the salvaged work before merge" contract as mika#1282's dirty-worktree rescue. The marker carries the operator-review requirement structurally.

### Edit 2 — open mika#1383 PRs as draft

Add `--draft` flag to the `gh pr create` invocation at `dispatch-lib.sh:1054-1059`. Mirrors mika#1282's convention. Operator un-drafts when ready to merge.

### Edit 3 (architect F1 BLOCKING) — ensure head commit matches Guard 2's `^wip\(` regex

Guard 2's defense-in-depth check parses `gh pr view --json commits --jq '.commits[-1].messageHeadline'` and matches against the anchored regex `^wip\(`. The mika#1383 path uses the pilot's existing commits as-is — whose head-commit headline is whatever the pilot wrote (typically a conventional-commit title like `fix(...)` or `feat(...)`, NOT `wip(...)`). Without Edit 3, Guard 2's conjunction (`isDraft AND ^wip\(`) still fails even after Edits 1+2 land — half the regression net is wasted.

**Mechanism:** before `gh pr create` (after the branch already has the pilot's commits + push), add an **empty marker commit** on top:

```bash
git -C "$WORKTREE_DIR" commit --allow-empty -m "wip(mika#1383): auto-PR-create rescue for ${REPO}#${ISSUE_NUM}

The pilot session committed and pushed but did not reach gh pr create
before its turn ended. dispatch-lib took ownership of the PR creation
per mika#1271 (content/workflow split). This marker commit signals the
rescue class so Guard 2's wip-rescue draft signature fires correctly.

The pilot's actual implementation work is in the commit(s) below this one."
```

Then `git push` the marker commit + invoke `gh pr create --draft ...`. The `commits[-1].messageHeadline` will be `wip(mika#1383): auto-PR-create rescue for ...` — Guard 2's `^wip\(` regex matches.

**Why a marker commit, not `git commit --amend`:** amending a pushed commit rewrites history and requires `--force-with-lease`. The marker commit is purely additive — no force-push, no history rewrite, preserves the pilot's commit shape. The cost is one extra commit in the PR's history — explicitly justified by the marker message.

**Position relative to Edits 1+2:** Edit 3 happens between the existing `git push` (pilot's commits already pushed) and the `gh pr create` call — a new `git push` is needed for the marker commit. Implementation outline §2-3 carries the sequencing.

### Edit 4 (absorbed from mika#1684, Prime-ratified 2026-06-30 ~16:02Z) — emit rescue PR body header

The mika#1383 path's current PR body (`Auto-created by dispatch-lib (mika#1383 structural completion gate)...`) does NOT contain the `## Auto-rescued PR (dispatch-lib recovery, class: ...)` header that qa-review's Step 1.5 keys on. As a result, qa-review classifies the PR as non-rescue → proceeds to standard review → approves (witnessed live on mika#PR1683 at 15:54:35Z).

**Mechanism:** replace the mika#1383 path's PR body construction (around `dispatch-lib.sh:1045-1075`) with the same structured rescue-body shape used by the mika#1282/mika#1396 rescue path at lines 2491-2508. The body must contain:

```markdown
## Auto-rescued PR (dispatch-lib recovery, class: commit-pushed-no-pr)

<!-- rescue-pipeline-verified: no -->

This PR was created by dispatch-lib's git-workflow recovery. The pilot session
committed and pushed but did not reach `gh pr create` before its turn ended.
dispatch-lib opened this PR from the existing branch.

**Auto-rescued PR.** Operator: verify pipeline completion, then either un-draft
this PR or set the marker above to `yes`.

### Recovery metadata
- Recovery class: `commit-pushed-no-pr`
- Pilot session: `${SESSION_ID:-unknown}`
- Turns: ${TURNS:-unknown}
- Cost: $${COST:-unknown}

Closes #${ISSUE_NUM}
```

This makes qa-review's Step 1.5 correctly detect the rescue header, evaluate `<!-- rescue-pipeline-verified: no -->`, see the PR is still draft (after Edit 2's `--draft` flag fix), and emit `hold[review]` per Step 1.5 §4's "Not verified" branch. **Operator must manually verify before un-drafting + merging.**

**Composition with Edits 1-3:** producer-side coherent fix — Edits 1 (marker), 2 (`--draft`), 3 (wip commit), 4 (body header) all together make the mika#1383 path's PR-open shape match the downstream consumer contracts (Guard 1 + Guard 2 + qa-review Step 1.5). Shipping any subset leaves a half-closed contract that's arguably worse than the current clean-bypass (e.g., a rescue-header body on a non-draft PR misleads).

## Implementation outline

1. **Locate the mika#1383 tail block** (`dispatch-lib.sh` around lines 1045-1080). Identify the exact `gh pr create` call and the line that appends `dispatch-lib (mika#1383): auto-created PR ...` to RESULT.

2. **Apply Edit 3 (marker commit) BEFORE `gh pr create`:** insert a `git commit --allow-empty -m "wip(mika#1383): auto-PR-create rescue for ${REPO}#${ISSUE_NUM}\n\n..."` step, followed by `git push origin "$BRANCH"`. Verify the push succeeds before proceeding to `gh pr create`. Fail-loud on push failure (the marker commit must reach origin for Guard 2 to see it).

3. **Apply Edit 2 (add `--draft`):** modify the `gh pr create` invocation to include `--draft` flag. Position: alongside `--repo`, `--base`, `--head`, `--title`, `--body` — alphabetical or grouped, no preference.

4. **Apply Edit 1 (emit marker):** after the `RESULT="${RESULT}` append block at line 1067, append a second line setting `RECOVERY_PENDING: true`. Use the same multi-line heredoc-style append shape that mika#1282 uses at lines 2524-2526 for visual consistency.

5. **Apply Edit 4 (rescue body header):** replace the mika#1383 path's PR body string with the heredoc-shaped rescue body per §Fix shape Edit 4. Set `RECOVERY_CLASS="commit-pushed-no-pr"` in the body literal (this path's class). Verify body contains both the `## Auto-rescued PR (dispatch-lib recovery, class: commit-pushed-no-pr)` header AND the `<!-- rescue-pipeline-verified: no -->` HTML comment marker.

6. **Verify mika#1282 path still works:** read lines 2491-2540 to confirm the existing block still emits `## Auto-rescued PR (dispatch-lib recovery, class: dirty-worktree)` header, `<!-- rescue-pipeline-verified: no -->` marker, `--draft`, `PR:`, `Draft PR (dispatch-lib recovery):`, and `RECOVERY_PENDING: true` — no regression. (AC3 + AC6 cover this.)

7. **Test surface** — add a dispatch-lib integration test (or extend an existing one in `skills/bundled/_shared/tests/` if the harness exists) that covers both rescue paths. Verify (a) both paths emit `RECOVERY_PENDING: true`, (b) both PRs open as draft, (c) the mika#1383 path's head commit headline matches `^wip\(`, (d) both PR bodies contain the `## Auto-rescued PR (dispatch-lib recovery, class: <class>)` header AND `<!-- rescue-pipeline-verified: no -->` marker (Edit 4 / AC6). Implementer first task: grep `skills/bundled/_shared/` for existing test scaffolding.

## Acceptance criteria

- **AC1** — `skills/bundled/_shared/dispatch-lib.sh:~1067` mika#1383 auto-PR-create tail emits `RECOVERY_PENDING: true` to RESULT after the descriptive `dispatch-lib (mika#1383): auto-created PR ...` line. Marker on its own line, no leading whitespace (mirror mika#1282's emission).

- **AC2** — `skills/bundled/_shared/dispatch-lib.sh:~1054` mika#1383 `gh pr create` invocation includes `--draft` flag. PR opens as draft (`isDraft: true` per `gh pr view --json isDraft`).

- **AC3** — Regression check: mika#1282 dirty-worktree rescue path (lines 2510-2540) continues to emit `RECOVERY_PENDING: true` AND open PRs as draft. Verified by reading the block in PR diff — no inadvertent edits.

- **AC4** — Integration test covers both paths producing all three Guard signals: (a) RESULT contains `RECOVERY_PENDING: true`, (b) PR opens as draft (`isDraft: true`), (c) head commit headline matches `^wip\(` regex. Test placement: dispatch-lib's existing test harness if any (e.g., `skills/bundled/_shared/tests/test-dispatch-lib.sh`), or a new test if the harness doesn't yet cover rescue scenarios. PR body must include test invocation + pass output. If no harness exists, AC4 is reduced to documented manual verification in PR body — implementer surfaces this in the PR description (architect F3 ratified path).

- **AC5 (architect F1)** — Edit 3 marker commit verification: after a mika#1383 rescue runs end-to-end, `gh pr view <N> --json commits --jq '.commits[-1].messageHeadline'` returns a string starting with `wip(mika#1383):`. Test or live-run evidence in PR body.

- **AC6 (Prime refinement, absorbed from mika#1684)** — Edit 4 PR body header verification: after a mika#1383 rescue runs end-to-end, `gh pr view <N> --json body` returns a body that contains BOTH the literal string `## Auto-rescued PR (dispatch-lib recovery, class: commit-pushed-no-pr)` AND the HTML comment `<!-- rescue-pipeline-verified: no -->`. qa-review's Step 1.5 detects the header and short-circuits to `hold[review]` rather than approving. Validate by replaying a synthetic webhook against qa-review's prompt logic OR by capturing live evidence in the next mika#1383-path PR's qa verdict. mika#1684 closes as subsumed when this AC passes.

## Out of scope

- **Engine-side guard for non-marker, non-draft PRs.** Would require qa-webhook architectural redesign (the guard contract is currently anchored to BOTH metadata flag + PR-shape signals; broader catch-all "any PR with `wip(` title regardless of draft state" would change the contract semantics). File a separate ticket if mika#1679's coverage proves insufficient post-deploy.
- **Migration of existing non-draft wip-rescue PRs.** PR #1678 + any in-flight non-draft rescues from before this fix lands remain operator-managed. Fix prevents future occurrences.
- **Unifying mika#1282 + mika#1383 rescue blocks into a shared helper.** The blocks already share structural shape; refactor only if further drift surfaces.

## Files involved

- `skills/bundled/_shared/dispatch-lib.sh` — Edit 1 (~line 1067) + Edit 2 (~line 1054) + Edit 3 (marker commit + push, between push and `gh pr create`) + Edit 4 (rescue body header in the PR body, ~line 1045-1075)
- `skills/bundled/_shared/tests/test-dispatch-lib.sh` (if exists) — AC4 + AC6 integration test
- No Rust/source-code changes; no schema migration; no engine guard changes

## Verification

- **Static:** read PR diff. Edit 2 visible at `gh pr create` call (the `--draft` flag added). Edit 1 visible at the mika#1383 tail RESULT append (the marker line added). mika#1282 block untouched.
- **Synthetic test (AC4):** invoke the dispatch-lib test harness with a mock pilot session that (a) commits + pushes but doesn't `gh pr create` (mika#1383 trigger), and (b) leaves dirty worktree (mika#1282 trigger). Verify both produce `RECOVERY_PENDING: true` in callback RESULT AND open draft PRs.
- **Live verification post-merge:** the next pilot session that hits either rescue path opens a draft PR with the marker. Confirm `gh pr view <N> --json isDraft` returns `true` AND `tasks.metadata` carries `unpushed_recovery_pending: true` via SQL: `SELECT json_extract(metadata, '$.unpushed_recovery_pending') FROM tasks WHERE id = <task>`.

## References

- mika#1613 (CLOSED) — parent fix (PR #1677, merged 13:48Z)
- mika#1282 — original dirty-worktree rescue, source of the marker + `--draft` + rescue-body convention
- mika#1383 — auto-PR-create structural completion (where these gaps originated)
- mika#PR1678 (MERGED) — bypassed case (mika#1645 impl, opened non-draft via mika#1383 path 14:05Z post-deploy)
- mika#PR1683 — layer-3 bypass evidence: mika-qa auto-approved at 15:54:35Z because the mika#1383 PR body lacked the `## Auto-rescued PR` rescue header (motivates Edit 4)
- mika#1684 (filed 16:00Z, absorbed here as Edit 4 per Mika Prime bearing ~16:02Z — close as subsumed when this lands)
- mika#1618 — qa-review Step 1.5 rescue-class detection (the consumer Edit 4 feeds)
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — Guard 1 + Guard 2 source
- `skills/bundled/qa-review/system_prompt.md:113-122` — Step 1.5 rescue-header detection (Edit 4's consumer)
- `skills/bundled/_shared/dispatch-lib.sh:1045-1080` — mika#1383 auto-PR-create tail (edit targets)
- `skills/bundled/_shared/dispatch-lib.sh:2491-2540` — mika#1282 dirty-worktree rescue (the convention to mirror — body shape at 2491-2508, draft + marker at 2510-2540)
- Mika Prime bearing 2026-06-30 ~16:02Z (session `00000000-...`): absorb mika#1684 as Edit 4 because the four edits are one producer-side contract-fix in four sites, not four independent fixes.
