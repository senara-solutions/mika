---
type: fix
issue: 1694
title: Substrate hygiene — worktree+branch debt audit/clean + auto-reap on PR close
status: draft
---

# Plan — mika#1694 worktree+branch debt hygiene

## Ticket

mika#1694 — three-layer fix for the worktree/branch debt accumulation pattern Vincent surfaced with hard evidence (14 worktrees, 3 dirty pilot work, 30+ orphan branches). Layer A audit surface, Layer B automated clean, Layer C structural auto-reap. AC1-AC6 defined.

## Problem

Every autonomous-loop dispatch creates a worktree at `mika-platform/.claude/worktrees/<sanitized-branch-slug>/mika/` (or the target sub-repo). The dispatch-lib's mika#1282 cleanup logic reaps the worktree on successful PR open — but wedge-failure paths (clippy-wedge, policy-deny, silent pilot death) leave them stranded. Once stranded, no mechanism removes them; disk usage grows; operator confusion grows ("why do I still have `fix-1204-dispatch-lib-post-flight-recovery` from days ago"); dirty worktrees rot silently with unsalvaged pilot work.

## Scope

**In scope (v1 ships):**

1. **Layer A** — `mika-platform/scripts/worktrees-audit` (invoked via `make worktrees-audit` from meta-repo root). Read-only enumeration: worktree path, branch, associated PR (state), dirty-flag. Non-zero exit if any dirty/orphan present.
2. **Layer B** — `mika-platform/scripts/worktrees-clean` (invoked via `make worktrees-clean`). For each worktree whose PR is MERGED or CLOSED: `git worktree remove` + `git branch -D` (local branch delete). For dirty worktrees: refuse to remove, surface loudly. For orphans (no PR found by branch name): list only unless `--orphans` flag passed.
3. **Layer C.1** — Auto-reap on `pull_request.closed` webhook. Mika-dev's `self-dev-webhook-qa` or a new `self-dev-webhook-pr-closed` handler receives the event; invokes `worktrees-clean` for the closed PR's branch specifically.
4. **Layer C.2** — Pre-commit hook at `mika-platform/.claude/hooks/check-worktree-hygiene.sh` — invoked by orchestrator-CC's PreToolUse on git commit. Asserts no dirty worktrees exist (or surfaces list). Non-blocking warning by default; loud enough that the operator sees it.
5. **Documentation** — `mika/docs/operator/worktree-hygiene.md` — command reference + recovery procedure for stranded pilots.
6. **Retroactive cleanup** — Run `make worktrees-clean` after implementation lands. Reduce the current 14 → ~4 worktrees.

**Out of scope:**

- Origin-branch cleanup (30+ stale remote branches). Some are referenced by closed PRs we want to keep for history. Separate operator decision.
- Cross-repo cleanup for mika-cloud/mika-skills worktrees. mika is the primary volume; those repos rarely see stranded worktrees.
- Aggressive dirty-worktree salvage (auto-commit + push). That's mika#1282's job on the failure path; this ticket doesn't extend salvage semantics, only cleans up after the dust settles.

## Repository shape (cross-repo touch)

**mika-platform (meta-repo):**
- `scripts/worktrees-audit` — new file
- `scripts/worktrees-clean` — new file
- `Makefile` — add `worktrees-audit`, `worktrees-clean` targets
- `.claude/hooks/check-worktree-hygiene.sh` — new file
- `.claude/settings.json` — wire the hook (existing PreToolUse pattern)

**mika:**
- `skills/bundled/self-dev-webhook-*` — new webhook handler for `pull_request.closed` OR extend an existing one (architect-bearing)
- `docs/operator/worktree-hygiene.md` — new file

The ticket is filed on mika but the primary write-path is mika-platform because worktrees live there. The webhook handler is mika-side because mika-dev owns webhook routing.

**Cross-repo PR pattern:** Primary PR against mika-platform (scripts + Makefile + hook + settings). Companion PR against mika (webhook handler + docs). Coordinate branch names across repos per meta-repo convention.

## Layer-by-layer design

### Layer A — `worktrees-audit`

Bash script. Invocation: `make worktrees-audit` from meta-repo root.

Output format (one line per worktree, whitespace-separated, machine-parseable):

```
<worktree-path> <branch-ref> <pr-number-or-none> <pr-state> <dirty-flag>
```

Algorithm:
1. `find .claude/worktrees/ -maxdepth 2 -name mika -o -name mika-cloud -o -name mika-skills -o -name claude-pilot-py -type d` → worktree candidates.
2. For each: `git -C <path> symbolic-ref --short HEAD` → branch.
3. `git -C <path> status --porcelain` → dirty if non-empty.
4. For each branch: `gh pr list --repo <derived-repo> --head <branch> --state all --limit 1 --json number,state` → PR number + state.
5. Emit line.

Exit code: 0 if all worktrees clean + all have MERGED/CLOSED PRs (no orphans). 1 otherwise (surfaces to operator).

### Layer B — `worktrees-clean`

Bash script. Invocation: `make worktrees-clean` from meta-repo root.

Algorithm:
1. Enumerate as Layer A does.
2. For each worktree where PR state ∈ {MERGED, CLOSED} AND worktree is clean:
   - `git -C <sub-repo> worktree remove <path>` (or `--force` if worktree metadata is stale — soft-error on that)
   - `git -C <sub-repo> branch -D <branch>` (local branch delete)
3. For each dirty worktree: print loudly (`✗ DIRTY: <path> — refusing to remove; run <recovery-procedure>`). Skip.
4. For each orphan (no PR found): print (`? ORPHAN: <path> — branch has no associated PR`). Skip unless `--orphans` flag passed.
5. Summary at end: `N removed, M dirty (skipped), K orphans (skipped)`.

Idempotent: subsequent invocations on already-cleaned state → no-op with success exit.

### Layer C.1 — Auto-reap webhook handler

Webhook event: `pull_request.closed` (fires on both merge and close-without-merge).

**Container-host filesystem boundary verification (F1 pre-implementation blocker)** — Before authoring the handler, verify mika-dev's `run_shell` can execute `git worktree remove` against the HOST filesystem where worktrees live (`/data/workspace/mika-platform/.claude/worktrees/`):
- Check `docker-compose.yml` and `Dockerfile.agent` for bind mounts of the workspace path.
- Grep existing `self-dev-webhook-*` handlers for existing filesystem reach against the same paths (mika#1282's cleanup logic in dispatch-lib is a positive precedent — it operates on the same worktree directory from the agent side).
- If mika-dev in dev mode runs as a host-process (not containerized) — confirmed by `MIKA_DEV_MODE=true` provisioning at `~/.mika/agents/mika-dev/` per mika/CLAUDE.md — direct `git worktree remove` works. In containerized production, requires bind mount.

If verification confirms filesystem reach: proceed with handler design below.
If verification fails: redesign Layer C.1 to emit an `audit_events` row (kind = `worktree_reap_requested`, payload includes PR number + head branch), consumed by a host-side cron OR operator-CC session running `worktrees-clean` in a scheduled loop. The webhook handler becomes an event-emitter, not a direct reaper.

Handler location:

**Option a** — extend `self-dev-webhook-qa`. Reuses existing infrastructure. But qa is scoped to review verdicts; adding cleanup mixes concerns.

**Option b (preferred, architect-ratified)** — new `self-dev-webhook-pr-closed` bundled skill. Clean separation, one purpose per handler. No existing pull_request.closed handler in mika-dev's manifest (verified via search: only `self-dev-webhook-qa` + `self-dev-webhook-ci` + generic `self-dev` fallback exist). New manifest, new keyword-trigger, new agent-allowlist entry in `MIKA_DEV_IDENTITY`.

**Option c** — dispatch-lib.sh function `_reap_worktree_for_pr()` invoked from the existing pull_request.closed pathway if one exists. Verify by grep of `pull_request` in existing handlers before selecting.

Preference: (b) — separate handler, clean boundaries.

Handler body:
1. Extract PR number + head branch from webhook payload.
2. Derive worktree path from branch name (use `scripts/derive-worktree-path`).
3. Verify PR state (should be closed).
4. Check worktree exists + is clean.
5. If clean + closed: invoke `worktrees-clean` for that specific worktree (single-worktree mode via new flag) OR replicate the same operations inline.
6. If dirty: emit a mika audit event (`worktree.reap_skipped.dirty`) so it surfaces in dashboard, do NOT remove.
7. If missing (already reaped): no-op, log.

Post-implementation smoke test: simulate a `pull_request.closed` event via `curl` to mika-gateway + verify worktree is removed within 30 seconds; if not, F1 verification was incomplete — surface + investigate.

### Layer C.2 — Pre-commit worktree-hygiene assertion

Location: `mika-platform/.claude/hooks/check-worktree-hygiene.sh`.

Hook trigger: `PreToolUse` on `Bash` commands matching `git commit` OR on `Edit`/`Write` to `docs/logs/*.md` (the handsoff-log write, per `feedback_pre_commit_split_criterion_before_investigating` pattern).

**Recursion guard (F2 required — first line of script):**
```bash
#!/bin/bash
# check-worktree-hygiene.sh — assert no dirty worktrees before orchestrator-CC commits.
#
# Recursion guard: this hook is scoped to orchestrator-CC sessions in the META-repo.
# Committing from inside a worktree is legitimate pilot/dispatch work — the hook must
# NOT flag the operator's own in-flight worktree work as dirty debt.
# If PWD is not the meta-repo root, exit 0 immediately.
[[ "$PWD" != "/data/workspace/mika-platform" ]] && exit 0
```

Behavior after the guard:
1. Invoke `worktrees-audit` in machine-readable mode.
2. If output shows dirty worktrees: emit warning to stderr with the list + a one-line resolution reminder ("run `make worktrees-clean` or salvage manually — see docs/operator/worktree-hygiene.md").
3. Exit non-zero (BLOCKING) on dirty worktrees — forces the operator to make a salvage-or-discard decision before committing.
4. Exit 0 (non-blocking) on orphans-only — surface them in stderr but don't block.

Wiring: `mika-platform/.claude/settings.json` PreToolUse entry, following the pattern of `check-question-routing.sh`.

## Deliverables (mapped to ACs)

| AC | Deliverable | File(s) |
|---|---|---|
| AC1 | `make worktrees-audit` command | `mika-platform/scripts/worktrees-audit` + `mika-platform/Makefile` target |
| AC2 | `make worktrees-clean` command | `mika-platform/scripts/worktrees-clean` + `Makefile` target |
| AC3 | Auto-reap on PR close | New `self-dev-webhook-pr-closed` skill (or extended existing) at `mika/skills/bundled/` — architect-bearing |
| AC4 | Pre-commit worktree assertion | `mika-platform/.claude/hooks/check-worktree-hygiene.sh` + `.claude/settings.json` wiring |
| AC5 | Documentation | `mika/docs/operator/worktree-hygiene.md` |
| AC6 | Retroactive cleanup verified | Post-implementation `make worktrees-audit` shows ≤4 worktrees |

## Implementation steps (dispatch order)

**Phase 1 — Layer A + B scripts** (mika-platform PR).
- Author `worktrees-audit` + `worktrees-clean` scripts.
- Add `Makefile` targets.
- Test manually on current state (14 worktrees). Verify audit output format, verify clean does not remove dirty/orphans.

**Phase 2 — Pre-commit hook** (mika-platform PR, same as Phase 1).
- Author `check-worktree-hygiene.sh`.
- Wire in `.claude/settings.json`.
- Manually verify hook fires on `git commit` in a session with a dirty worktree present.

**Phase 3 — Retroactive cleanup + PR ready** (mika-platform PR merge candidate).
- Run `make worktrees-clean` to reduce current 14 → ~4.
- Commit any salvage decisions on dirty worktrees BEFORE running clean.
- Merge Phase 1+2 PR.

**Phase 4 — Webhook handler** (mika PR, companion).
- Author `self-dev-webhook-pr-closed` (or the architect-selected option).
- Wire skill.toml, add to identity allowlists (mika-dev at minimum).
- Add test scenario in mika-dev calibration for the new handler.

**Phase 5 — Documentation** (mika PR, same as Phase 4).
- Author `docs/operator/worktree-hygiene.md`.
- Cross-link from mika/CLAUDE.md operator section.

## Verification

- `make worktrees-audit` on current state prints all 14 worktrees with correct branch+PR-state annotations. Exit non-zero.
- `make worktrees-clean --dry-run` (if flag added) previews correct removal set.
- After actual clean: audit shows ≤4 worktrees, exit 0.
- Pre-commit hook fires on Bash git commit with a dirty worktree present. Non-blocking OR blocking per architect.
- Webhook handler test: simulate a `pull_request.closed` event via `curl` to gateway + verify worktree is removed within N seconds.
- `cargo test -p mika-agent` for any changes to well-known agent allowlists or scenario definitions — no regressions.

## Risks

1. **Cross-repo coordination.** Two PRs (mika-platform + mika). If one lands without the other, the loop is functional but partial. Mitigation: land mika-platform PR first (audit + clean scripts work standalone). mika PR is enhancement.
2. **Handler double-fire.** If both mika#1282's success-path cleanup AND the new pull_request.closed handler fire for the same worktree, the second is a no-op (already removed) — soft error. Verify handler is idempotent.
3. **Dirty worktree recovery flow.** The pre-commit hook flags dirty worktrees but doesn't remove them. Operator must salvage. Docs must be crystal-clear on what "salvage" means for each of the failure classes (clippy-wedge, policy-deny, orphan). Insufficient docs = operator continues to accumulate dirty worktrees.
4. **False positive orphans.** If a branch exists locally but the corresponding PR was renamed/rebased, the PR-lookup may fail. Fallback: `gh pr list --head <branch>` returns empty even for merged PRs. Consider also probing `gh api repos/<repo>/pulls?head=<owner>:<branch>&state=all` for archived PRs.
5. **Hook cadence.** Firing the hook on every `git commit` might spam. Rate-limit or dedupe to once per session? Or fire only on commit to main/orchestrator-CC session's own commits, not on worktree-internal commits (which would recursively check themselves)? Architect judgment.

## Acceptance criteria

Transcribed verbatim from mika#1694. This mika-scoped PR delivers the **mika-side** subset (AC3 webhook handler + AC5 docs); AC1/AC2/AC4/AC6 are the companion **mika-platform** PR's responsibility (scripts + Makefile + pre-commit hook + retroactive cleanup), per the cross-repo split in § Repository shape.

- [ ] **AC1** — `make worktrees-audit` exists. Lists every worktree with: branch name, PR number (or `<no PR>`), PR state, dirty-flag. Exit 0 on clean state, non-zero on dirty/orphan presence. *(mika-platform companion PR)*
- [ ] **AC2** — `make worktrees-clean` exists. Removes worktrees whose PRs are MERGED or CLOSED. Refuses to remove dirty worktrees. Lists orphans without removing unless `--orphans` passed. *(mika-platform companion PR)*
- [ ] **AC3** — Git hook (or webhook handler) wired on `pull_request.closed`: invokes the cleanup for that specific worktree. Located in dispatch-lib OR a new hook script — architect-bearing. *(mika-side — this PR)*
- [ ] **AC4** — Operator-CC handsoff pre-commit hook: asserts no dirty worktrees. Surfaces them loudly so the operator can decide salvage/discard. Located in `.claude/hooks/` or similar. *(mika-platform companion PR)*
- [ ] **AC5** — Documentation in `docs/operator/worktree-hygiene.md` (NEW) explaining the audit + clean commands + when to run + recovery if a worktree gets stranded mid-pilot. *(mika-side — this PR)*
- [ ] **AC6** — Post-implementation, the existing 14 worktrees are reduced to ~4 (main + active PRs). Verified by `make worktrees-audit`. *(mika-platform companion PR — retroactive cleanup)*

## Out of scope (repeated)

- 30+ stale origin branches — separate operator decision (some are referenced by closed PRs for history).
- Cross-repo cleanup for mika-cloud/mika-skills worktrees.
- Extended salvage semantics for dirty worktrees.

## References

- mika#1685 (LANDED + DEPLOYED 2026-07-01) — modal wedge cause fix (prevents future dirty worktrees from clippy-hook rejection)
- mika#1679 (LANDED + DEPLOYED 2026-07-01) — dispatch-lib recovery guards (prevents non-draft rescue PRs from bypassing qa)
- mika#1282 — original dirty-worktree rescue (this ticket cleans up AFTER 1282's post-flight recovery does its thing)
- mika#1686 — permission-policy class question (parallel loop-slowdown; not blocking this)
- mika#1696 — wedge-day epic (parent)
- Vincent's frustration quote 2026-06-30 17:25Z — surfaced in ticket body
- `mika-platform/.claude/worktrees/` — the target directory
- `mika-platform/scripts/derive-worktree-path` — canonical path derivation (must use this)
- `mika/skills/bundled/_shared/dispatch-lib.sh` — where mika#1282 lives
- `mika-platform/CLAUDE.md` § Development Workflow → Automatic worktree isolation — canonical worktree convention
