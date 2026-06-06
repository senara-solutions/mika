# Plan: fix(skills) — stop skill-install from clobbering worktree `.claude/commands/` on deploy (mika#1415)

> **Grooming history:** Direct architect first-pass (`mika ask --agent mika-arch`, session `7ab4acb4-e88f-4b43-bc32-fa3952d5bf2f`) → ITERATE (F1 Phase 0 Pin requirements, F2 AC4 scope split) → revisions applied → second-pass GROOMED (same session).
>
> **Routing note:** This plan was groomed via direct architect invocation rather than the canonical `/mika-groom-ticket` spawn workflow. Two independent `/mika-groom-ticket mika#1415` spawn attempts on 2026-06-06 reproduced the same bootstrap-hang on this ticket. The artifact contract matches the standard groom output; the means is non-standard.

## Problem

After mika#1255 shipped the polymorphic `/mika` command to `origin/main`, the skill-install pipeline (invoked on every `make deploy`) writes the **OLD pre-#1255 meta-repo dispatcher version** of `mika/.claude/commands/mika.md` (272 lines, the exact clobber #1255 eliminated) back into every existing worktree's working directory. It also writes 18 sibling `.claude/commands/*.md` files into each worktree.

**This re-creates the clobber #1255 was meant to eliminate, on every deploy.**

## Evidence (2026-06-05 worktree survey, 23:00Z)

17 worktrees total at `/data/workspace/mika-platform/.claude/worktrees/*/mika`:

- 4 clean
- 13 with the systematic 18-untracked-files pattern (the polymorphic command-set)
- 3 of the 13 additionally had `.claude/commands/mika.md` modified from polymorphic HEAD back to the 272-line meta-repo dispatcher

**Universal across in-flight worktrees; regenerated each deploy.**

## Approach — Phase 0 Pin → fix-shape commitment → implementation

### Phase 0 — Pin (implementer's FIRST work step, gates fix-shape commitment)

Before writing any fix code, the implementer must produce:

#### Phase 0.A — Identify the propagation code path

Grep + trace to find the **specific file(s) and function(s)** in the deploy chain that write `.claude/commands/*.md` into paths matching `.claude/worktrees/`.

Suggested starting points (not authoritative — pin must surface the actual path):

- `scripts/mika-platform-agents-tmux` (skill-install invocation per agent)
- `mika skills install` / `mika skills update` (the `mika-skills` repo or mika core's skill install logic)
- `Makefile`'s `deploy` target chain (mika-platform/Makefile → mika/Makefile → ...)
- `scripts/mika-platform-deploy-preflight` (called before deploy)

**Expected output:** a one-paragraph "Phase 0 Pin" section in the PR description (or in this plan as an addendum commit) naming:
- The file path
- The function or shell block
- The grep command + matching output that established the citation

#### Phase 0.B — Commit on dispatch-lib's second propagation surface

mika#1414's plan documents that dispatch-lib re-copies `.claude/commands/` from `$PLATFORM_DIR` after the rebase (per #1414's plan around line 358). This is a **per-dispatch** propagation surface distinct from the deploy-time one this ticket targets.

Implementer must commit to one of three positions, with rationale:

1. **Subsumed:** dispatch-lib's per-dispatch copy is correct behavior because the worktree was clean before the dispatch; fixing the deploy-time clobber means dispatch-lib's copy will have nothing stale to re-introduce.
2. **Separate:** dispatch-lib's per-dispatch copy independently introduces the clobber and must also be addressed (either in this ticket's scope or as a follow-up).
3. **Scope-out with rationale:** dispatch-lib's per-dispatch copy is intentional and stays as-is; the explanation must be explicit.

This commit goes in the Phase 0 Pin section of the PR description (or plan addendum).

#### Phase 0.C — Fix-shape commitment based on pinned evidence

After Phase 0.A and 0.B, the implementer commits to ONE of three fix shapes (the architect's first-pass advisory read points to Shape 1, but the pin is authoritative):

- **Shape 1 — Skip-worktree detection.** The propagation code walks the workspace and writes to worktree `.claude/commands/`. Add a worktree-detection guard via `git rev-parse --git-dir` ≠ `git rev-parse --git-common-dir`; skip when inside a worktree.
- **Shape 2 — Canonical-path-only writes.** The propagation code writes to multiple targets including worktrees; constrain the target set to canonical install paths (`~/.mika/agents/<agent>/skills/` and/or the main checkout's `.claude/commands/`) and drop the workspace walk.
- **Shape 3 — Generator-based fix.** If skill-install regenerates the command-set from a template + interpolation (rather than copying from a static source), the bug is upstream — in the template or its inputs. Fix is in the generator's input.

### Implementation phase (gated on Phase 0 commitment)

The implementer writes the fix per the chosen shape. Implementation must NOT precede Phase 0 commitment.

Implementation considerations:

- Changes are likely small (a worktree-detection guard, a path filter, or a template update — depending on shape)
- The fix must NOT break canonical-path installs (agents at `~/.mika/agents/<agent>/skills/`)
- If Shape 1 or 2, the fix should preserve the install behavior for the main checkout's `.claude/commands/`
- If dispatch-lib's second propagation surface (Phase 0.B) is "Separate," that fix is also implemented in this PR (cite explicitly) OR filed as a peer ticket before merge

### Verification

Local reproducer (matches AC3):

```bash
# Step 1: pick an existing worktree
WT=/data/workspace/mika-platform/.claude/worktrees/<any>/mika

# Step 2: ensure clean baseline
git -C "$WT" status --porcelain  # expect empty
git -C "$WT" stash --include-untracked  # if not empty, stash first

# Step 3: run deploy
cd /data/workspace/mika-platform
make deploy

# Step 4: verify worktree remains clean (this is the regression gate)
git -C "$WT" status --porcelain  # expect empty AFTER fix
```

Pre-fix: step 4 outputs 13+ lines (M `.claude/commands/mika.md`, ?? sibling files).
Post-fix: step 4 outputs zero lines.

Automate as a shell script committed under `scripts/` or `tests/` for regression coverage. Per AC4's split-to-follow-up decision, the CI gate variant is NOT part of this PR.

### Documentation

Per AC5:

- Update `mika-platform/CLAUDE.md` (or the skills doc) to name the canonical install path(s) explicitly.
- Add a note: "Skill installs land at `~/.mika/agents/<agent>/skills/` and the main checkout's `.claude/commands/`. Worktree `.claude/commands/` directories are NOT install targets; per-worktree command state is sourced from the worktree's branch HEAD."

## Acceptance criteria (reconciled with second-pass GROOMED scope)

- [ ] **AC1** — Phase 0 Pin complete (Phase 0.A propagation path identified with file:function + grep citation; Phase 0.B dispatch-lib position committed: subsumed/separate/scope-out with rationale)
- [ ] **AC2** — Fix shape committed per Phase 0.C; implementation matches commitment; fix lands in single PR
- [ ] **AC3** — Local reproducer script: clean worktree + `make deploy` → worktree stays clean (committed as a script under `scripts/` or `tests/`)
- [ ] **AC5** — Canonical install paths documented in `CLAUDE.md` or skills doc

(AC4 — CI gate for worktree dirtying post-deploy — split to follow-up ticket per second-pass scope ruling; filed after this PR merges.)

## Observability — n=1 on the bootstrap-hang × ticket-body interaction class

(Architect-drafted neutral observation language, captured for future class-binding):

Two independent `/mika-groom-ticket mika#1415` spawn attempts (`ab91bf0d-...` and `10ee495f-...`) on 2026-06-06 reproduced the same bootstrap-hang pattern: claude binary alive (PID present), zero subprocesses spawned (verified via `pgrep -P <claude_pid>`), tmux pane empty after 60-second observation window, IO at idle-Bun-noise rate (~10KB/10s). Same launcher (`/mika-groom-ticket`) worked cleanly on mika#1414 during the same morning session, producing subprocess activity and a complete groom artifact. Variable isolated to ticket-specific interaction, not launcher-generic. Class not yet bound (n=1); awaiting n=2 before filing a substrate ticket per discipline. This ticket's body serves as the provenance anchor for the first instance.

## Out of scope

- The CI gate for worktree-dirtying detection (split to follow-up ticket per AC4 scope ruling).
- Any unrelated skill-install refactor.
- Reviewing or modifying the skills marketplace install flow (`mika skills install <user-skill>`); this plan's fix targets ONLY the deploy-time bundled-skill propagation that walks the workspace.

## Phase 0 Pin — RESULT (implementer, 2026-06-06)

> **Premise correction (operator-confirmed re-scope).** The groomed ticket framed this as a *deploy-time skill-install* bug. The pin disproves that premise. Issue re-titled to `fix(dispatch-lib): worktree-setup clobbers sub-repo .claude/commands (#1255 regression on every dispatch)`.

**Phase 0.A — propagation path (pinned with evidence):**

`mika/skills/bundled/_shared/dispatch-lib.sh:359` (pre-fix), inside `_set_up_worktree()`:
```bash
cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"
```
Exhaustive grep across `mika`, `mika-platform`, `mika-skills` + mika-core Rust confirms this is the **only** writer of `.claude/commands/` into a worktree. Mechanism reproduces the survey symptom exactly: meta-repo `.claude/commands/` (21 files) minus the worktree's 4 tracked files = **18 untracked siblings**; the 3 overlapping names (`mika.md` 72→260 lines, `mika-issue.md`, `mika-issues.md`) are **clobbered**.

**Phase 0.B — the deploy-time premise is unsupported:** every `make deploy` sub-step traced (`build`/`install-mika`/`install-claude-pilot`/`install-skills`/`restart`/`check-ngrok`); none write `.claude/commands`. `mika skills update` has zero `.claude`/`commands`/`worktree` references. There is no deploy-time/skill-install surface. The cp fires at **worktree-creation during a dispatch**, not on deploy. The "two distinct surfaces" model collapses to one — and it is `dispatch-lib.sh`, the file mika#1414 also touches.

**Phase 0.C — fix shape: Shape 1 (path-redirect), operator-confirmed.** Replace the blanket `cp -r` with `_seed_worktree_slash_commands()` enforcing two invariants: (1) never overwrite a command the worktree already tracks (preserves #1255 polymorphic `/mika`); (2) shield meta-only copies via the common-dir `.git/info/exclude` so the worktree stays git-clean (the per-worktree `$GIT_DIR/info/exclude` is *not* honored for status — verified empirically). Meta orchestration commands stay available for the inner session (#1173).

**mika#1414 boundary (non-overlap):** #1414 owns the **pre-rebase** dirty-state cleanup + rebase guard (the mika#1301 block, ~L294–311, plus its `_clean_worktree_for_rebase` helper). #1415 owns the **post-rebase** command-seed (the cp block + `_seed_worktree_slash_commands`). Same function (`_set_up_worktree`), disjoint line ranges and disjoint helpers. #1415 removes the *cause* of dirty worktrees; #1414 stays the *defensive* net.

## Related

- mika#1255 — the polymorphic /mika fix this propagation undoes every deploy.
- mika#1414 — sibling defensive fix in dispatch-lib (already GROOMED, plan at `mika/docs/plans/2026-06-06-001-fix-dispatch-lib-resume-dirty-worktree-plan.md`, session `323e8914`).
- mika#1282 — content/workflow split contract (the recovery surface dispatch-lib provides for in-flight pilot work).
- mika#1407 — peer substrate fix (dispatch-lib classifier — shipped).
- milestone-30 — Loop Trustworthiness — observability → stability.
