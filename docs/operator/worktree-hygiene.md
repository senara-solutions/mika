# Worktree hygiene

> Substrate-hygiene runbook for the autonomous-loop dispatch worktrees
> (mika#1694). Covers the automated PR-close reaper, the operator audit/clean
> commands, and recovery when a worktree gets stranded mid-pilot.

## The debt this closes

Every autonomous-loop dispatch creates a git worktree at
`mika-platform/.claude/worktrees/<sanitized-branch-slug>/<repo>/`. Without a reap
mechanism, worktrees accumulate indefinitely: disk fills, `git worktree list`
grows unreadable, and dirty worktrees rot silently when a pilot wedges. On
2026-06-30 the operator observed **14 worktrees** (up from ~4), 3 of them dirty
with unsalvaged pilot work — the frustration that motivated this runbook
("is the platform going to be stable one day or what?").

Three layers keep it flat:

| Layer | What | Where | Trigger |
|-------|------|-------|---------|
| **A — audit** | Read-only visibility surface | `make worktrees-audit` (mika-platform) | Operator, on demand |
| **B — clean** | Bulk removal of merged/closed worktrees | `make worktrees-clean` (mika-platform) | Operator, on demand |
| **C — reap** | Automatic per-PR removal on close | mika-spirit structural handler | `pull_request.closed` webhook |

Layer C prevents new debt; layers A/B clean up whatever slips through (a webhook
missed while mika-spirit was down, a worktree created outside the loop).

## Layer C — the automatic reaper (this repo)

When a `pull_request.closed` webhook arrives, the gateway routes it to `mika-dev`
as a `[GitHub] PR closed: …` message. Before the LLM turn, the mika-spirit
structural handler [`server::worktree_reaper`](../../crates/mika-agent/src/server/worktree_reaper.rs)
reaps that branch's dispatch worktree:

1. Enumerates the git worktree registry (`git worktree list --porcelain`) and
   matches the branch from the webhook — the registry is ground truth, so there
   is no path-derivation drift.
2. **Refuses** to touch any worktree outside `.claude/worktrees/` (never the
   primary checkout or a human's manual worktree).
3. **Refuses** to remove a **dirty** worktree — it flags it with a
   `worktree.reap_skipped_dirty` audit event and leaves it for you to salvage.
4. On a clean worktree: `git worktree remove` + `git branch -D`, then emits a
   `worktree.reaped` audit event.

It fires on **both** merged and closed-without-merge PRs — both are terminal for
the branch's worktree. It is a pure side effect: the existing milestone-advance
and acknowledgement flow is unchanged.

### Why a structural handler, not a bundled skill

A webhook-triggered LLM turn cannot reliably reach the host worktree filesystem
(`self-dev-webhook-ci` Rule 5: `run_shell`/`write_agent_file` on webhook turns
are sandboxed to the agent home). LLM-driven reaping would also be
non-deterministic. The structural handler runs in the mika-spirit process
(host-side in dev mode) with the same filesystem reach `dispatch-lib.sh` uses for
`git worktree remove`, and is deterministic + unit-tested.

### Configuration

| Env var | Default | Effect |
|---------|---------|--------|
| `MIKA_WORKTREE_REAP_REPO_DIR` | `/data/workspace/mika-platform/mika` | The mika checkout used to enumerate worktrees. Set an empty value to disable the reaper. |

The reaper is a **silent no-op** when the repo dir is absent or is not a git
checkout — the correct behavior in containerized production, where dispatch
worktrees do not live on the agent's filesystem.

### Observability

```bash
# Worktrees reaped on PR close
grep 'worktree.reaped' "$MIKA_SPIRIT_LOG_FILE"

# Dirty worktrees the reaper refused to remove — these need operator salvage
grep 'worktree.reap_skipped_dirty' "$MIKA_SPIRIT_LOG_FILE"

# Reap attempts that failed at the git-remove stage
grep 'worktree.reap_failed' "$MIKA_SPIRIT_LOG_FILE"
```

All three are also queryable as `audit_events` rows (target key
`worktree:<path>`).

## Layers A & B — operator commands (companion mika-platform PR)

> These land in the companion **mika-platform** PR (AC1/AC2). Run them from the
> meta-repo root.

```bash
# Layer A — visibility. Lists every worktree with branch, PR number, PR state,
# and dirty-flag. Exit 0 on a clean state; non-zero if any dirty/orphan present.
make worktrees-audit

# Layer B — cleanup. Removes worktrees whose PR is MERGED or CLOSED. Refuses to
# remove dirty worktrees. Lists orphans (no PR) without removing them unless you
# pass --orphans.
make worktrees-clean
```

Run `worktrees-audit` whenever you want to see what is accumulating; run
`worktrees-clean` whenever you want a tidy state. Both are safe to run
repeatedly — cleanup is idempotent.

## Recovery — a worktree got stranded mid-pilot

A worktree is **stranded** when its pilot wedged (clippy-wedge, policy-deny,
silent pilot death) and left uncommitted work. The reaper and `worktrees-clean`
both **refuse** to remove it, so nothing is lost silently. To recover:

1. **Inspect** what is there:
   ```bash
   git -C .claude/worktrees/<slug>/<repo> status
   git -C .claude/worktrees/<slug>/<repo> diff
   ```
2. **Decide salvage or discard:**
   - **Salvage** — the changes are worth keeping. Commit them on the branch and
     push, so mika#1282 post-flight recovery / your PR carries them:
     ```bash
     git -C .claude/worktrees/<slug>/<repo> add -A
     git -C .claude/worktrees/<slug>/<repo> commit -m "wip(<repo>#<n>): salvaged pilot work"
     git -C .claude/worktrees/<slug>/<repo> push -u origin <branch>
     ```
     Once the resulting PR closes, the reaper removes the (now clean) worktree
     automatically.
   - **Discard** — the changes are garbage. Reset, then let the reaper or
     `worktrees-clean` remove it:
     ```bash
     git -C .claude/worktrees/<slug>/<repo> checkout -- .
     git -C .claude/worktrees/<slug>/<repo> clean -fd
     ```
3. **Force-remove** a worktree you have finished with manually:
   ```bash
   git -C <repo> worktree remove --force .claude/worktrees/<slug>/<repo>
   git -C <repo> branch -D <branch>
   ```

## Out of scope

- **Stale origin branches.** 30+ months-old remote branches are a separate
  operator decision — some are referenced by closed PRs kept for history. Not
  reaped here.
- **Cross-repo worktrees** for `mika-cloud` / `mika-skills`. `mika` is the
  primary volume; the reaper keys off events routed to `mika-dev`.
- **Extended dirty-worktree salvage** (auto-commit + push). That is mika#1282's
  job on the failure path; this runbook cleans up after the dust settles.
