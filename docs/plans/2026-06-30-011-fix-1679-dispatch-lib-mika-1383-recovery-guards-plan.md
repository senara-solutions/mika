---
issue: 1679
type: fix
date: 2026-06-30
---

# Plan — fix(dispatch-lib): mika#1383 auto-PR-create path bypasses mika#1613 recovery-pending guards (mika#1679)

## Problem

mika#1613's fix (PR #1677, merged + deployed 2026-06-30 13:48Z) added two qa-webhook guards in `skills/bundled/self-dev-webhook-qa/system_prompt.md`:
- **Guard 1** — checks `unpushed_recovery_pending: true` task metadata flag (set when callback parses `RECOVERY_PENDING: true` from RESULT).
- **Guard 2** — `isDraft AND ^wip\(` conjunction check.

**Both guards bypass for mika#1383's auto-PR-create path:**

1. **mika#1383 doesn't emit `RECOVERY_PENDING: true` marker.** The marker is only emitted in the mika#1282 dirty-worktree rescue block at `skills/bundled/_shared/dispatch-lib.sh:2524-2526`. The mika#1383 auto-PR-create tail at `dispatch-lib.sh:1067` only appends a descriptive `dispatch-lib (mika#1383): auto-created PR ...` line, no marker. → Guard 1 doesn't fire.

2. **mika#1383 opens PRs as non-draft.** The `gh pr create` invocation at `dispatch-lib.sh:1054-1059` lacks `--draft`. → `isDraft = false` → Guard 2's conjunction (`isDraft = true AND ^wip\(`) fails. PR opens normal, eligible for auto-merge.

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

## Implementation outline

1. **Locate the mika#1383 tail block** (`dispatch-lib.sh` around lines 1045-1080). Identify the exact `gh pr create` call and the line that appends `dispatch-lib (mika#1383): auto-created PR ...` to RESULT.

2. **Apply Edit 3 (marker commit) BEFORE `gh pr create`:** insert a `git commit --allow-empty -m "wip(mika#1383): auto-PR-create rescue for ${REPO}#${ISSUE_NUM}\n\n..."` step, followed by `git push origin "$BRANCH"`. Verify the push succeeds before proceeding to `gh pr create`. Fail-loud on push failure (the marker commit must reach origin for Guard 2 to see it).

3. **Apply Edit 2 (add `--draft`):** modify the `gh pr create` invocation to include `--draft` flag. Position: alongside `--repo`, `--base`, `--head`, `--title`, `--body` — alphabetical or grouped, no preference.

4. **Apply Edit 1 (emit marker):** after the `RESULT="${RESULT}` append block at line 1067, append a second line setting `RECOVERY_PENDING: true`. Use the same multi-line heredoc-style append shape that mika#1282 uses at lines 2524-2526 for visual consistency.

5. **Verify mika#1282 path still works:** read lines 2510-2540 to confirm the existing block still emits `PR:`, `Draft PR (dispatch-lib recovery):`, and `RECOVERY_PENDING: true` — no regression. (Acceptance criteria AC3 covers this.)

6. **Test surface** — add a dispatch-lib integration test (or extend an existing one in `skills/bundled/_shared/tests/` if the harness exists) that covers both rescue paths. Verify (a) both paths emit `RECOVERY_PENDING: true`, (b) both PRs open as draft, (c) the mika#1383 path's head commit headline matches `^wip\(`. Implementer first task: grep `skills/bundled/_shared/` for existing test scaffolding.

## Acceptance criteria

- **AC1** — `skills/bundled/_shared/dispatch-lib.sh:~1067` mika#1383 auto-PR-create tail emits `RECOVERY_PENDING: true` to RESULT after the descriptive `dispatch-lib (mika#1383): auto-created PR ...` line. Marker on its own line, no leading whitespace (mirror mika#1282's emission).

- **AC2** — `skills/bundled/_shared/dispatch-lib.sh:~1054` mika#1383 `gh pr create` invocation includes `--draft` flag. PR opens as draft (`isDraft: true` per `gh pr view --json isDraft`).

- **AC3** — Regression check: mika#1282 dirty-worktree rescue path (lines 2510-2540) continues to emit `RECOVERY_PENDING: true` AND open PRs as draft. Verified by reading the block in PR diff — no inadvertent edits.

- **AC4** — Integration test covers both paths producing all three Guard signals: (a) RESULT contains `RECOVERY_PENDING: true`, (b) PR opens as draft (`isDraft: true`), (c) head commit headline matches `^wip\(` regex. Test placement: dispatch-lib's existing test harness if any (e.g., `skills/bundled/_shared/tests/test-dispatch-lib.sh`), or a new test if the harness doesn't yet cover rescue scenarios. PR body must include test invocation + pass output. If no harness exists, AC4 is reduced to documented manual verification in PR body — implementer surfaces this in the PR description (architect F3 ratified path).

- **AC5 (architect F1)** — Edit 3 marker commit verification: after a mika#1383 rescue runs end-to-end, `gh pr view <N> --json commits --jq '.commits[-1].messageHeadline'` returns a string starting with `wip(mika#1383):`. Test or live-run evidence in PR body.

## Out of scope

- **Engine-side guard for non-marker, non-draft PRs.** Would require qa-webhook architectural redesign (the guard contract is currently anchored to BOTH metadata flag + PR-shape signals; broader catch-all "any PR with `wip(` title regardless of draft state" would change the contract semantics). File a separate ticket if mika#1679's coverage proves insufficient post-deploy.
- **Migration of existing non-draft wip-rescue PRs.** PR #1678 + any in-flight non-draft rescues from before this fix lands remain operator-managed. Fix prevents future occurrences.
- **Unifying mika#1282 + mika#1383 rescue blocks into a shared helper.** The blocks already share structural shape; refactor only if further drift surfaces.

## Files involved

- `skills/bundled/_shared/dispatch-lib.sh` — Edit 1 (~line 1067) + Edit 2 (~line 1054)
- `skills/bundled/_shared/tests/test-dispatch-lib.sh` (if exists) — AC4 integration test
- No Rust/source-code changes; no schema migration; no engine guard changes

## Verification

- **Static:** read PR diff. Edit 2 visible at `gh pr create` call (the `--draft` flag added). Edit 1 visible at the mika#1383 tail RESULT append (the marker line added). mika#1282 block untouched.
- **Synthetic test (AC4):** invoke the dispatch-lib test harness with a mock pilot session that (a) commits + pushes but doesn't `gh pr create` (mika#1383 trigger), and (b) leaves dirty worktree (mika#1282 trigger). Verify both produce `RECOVERY_PENDING: true` in callback RESULT AND open draft PRs.
- **Live verification post-merge:** the next pilot session that hits either rescue path opens a draft PR with the marker. Confirm `gh pr view <N> --json isDraft` returns `true` AND `tasks.metadata` carries `unpushed_recovery_pending: true` via SQL: `SELECT json_extract(metadata, '$.unpushed_recovery_pending') FROM tasks WHERE id = <task>`.

## References

- mika#1613 (CLOSED) — parent fix (PR #1677, merged 13:48Z)
- mika#1282 — original dirty-worktree rescue, source of the marker + `--draft` convention
- mika#1383 — auto-PR-create structural completion (where this gap originated)
- mika#PR1678 (MERGED) — bypassed case used as hard evidence (mika#1645 impl, opened non-draft via mika#1383 path 14:05Z post-deploy)
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — the guard logic this fix completes coverage for
- `skills/bundled/_shared/dispatch-lib.sh:1045-1080` — mika#1383 auto-PR-create tail (edit targets)
- `skills/bundled/_shared/dispatch-lib.sh:2510-2540` — mika#1282 dirty-worktree rescue (the convention to mirror)
