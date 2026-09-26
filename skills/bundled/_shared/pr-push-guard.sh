#!/usr/bin/env bash
# Guarded push for PR-branch pilots (mika#2520).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE FAILURE THIS CLOSES
#
# 2026-09-24, night. The `resolve-pr-conflicts` pilot dispatched for #2435 (task
# 357bb36e) REWOUND `origin/main` from 24f25e99 (#2489, merged 19:30:25Z) back
# to e851d72b (#2514). Restored by fast-forward under operator GO.
#
#   Bypassed rule violations for refs/heads/main:
#   Changes must be made through a pull request.
#   Cannot force-push to this branch
#    + 24f25e99...e851d72b main -> main (forced update)
#
# Step 6 of the skill's prompt was a BARE `git push --force-with-lease`. A bare
# push from a worktree lands wherever `push.default` and the local refs say it
# should, and the pilot identity carries an admin bypass — so a bare force-push
# is a rewind of `main` one character away.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE BARE FORM IS BROKEN ON TWO INDEPENDENT AXES, NOT ONE
#
# The ticket names the missing refspec — *where* the push lands. There is a
# second half it does not name, and it matters just as much: `--force-with-lease`
# with no `=<ref>:<expect>` compares against the local remote-tracking ref
# (`refs/remotes/origin/<branch>`). Step 1 of the same prompt is `git fetch
# origin`, which refreshes exactly that ref.
#
#   From the instant step 1 runs, the step-6 lease protects nothing: it compares
#   the remote against a snapshot of the remote taken after the fact.
#
# So the lease SHA is captured BEFORE the pilot session starts and carried
# through as a literal. A pilot `git fetch` cannot void it. This is strictly
# stronger than dispatch-lib's `--force-with-lease=$BRANCH:origin/$BRANCH`.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE ASYMMETRY THAT DECIDES EVERY REFUSAL'S DIRECTION
#
#   A refusal in error costs one dispatch — visible, recoverable, bounded.
#   A pass in error can rewind a protected branch — irreversible.
#
# Therefore EVERY unreadable signal REFUSES. This is the INVERSE of the mika#2420
# reaper's fail-safe rule (where an unreadable signal *keeps*), and the inversion
# is reasoned: there the action destroyed work, here the action IS the remote
# write. The trade-off is local; it does not transport.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE PRECEDENT THIS APPLIES (it is not invented here)
#
# mika#1318 -> mika#1407 removed the push from the dev-groom pilot and made
# `_push_branch` the single push site for those dispatches, after a dev-groom
# pilot ran `git push --force-with-lease` from inside its worktree and destroyed
# substrate-fix work on the remote. `resolve-pr-conflicts` is the sibling skill
# that never received that treatment. Same remedy, applied.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY A SHARED FILE RATHER THAN CODE INSIDE run.sh
#
# `run.sh` carries top-level imperative code (`set -e`, a trap, a stdin read) —
# it is not sourceable, so nothing inside it is testable without running it for
# real. `_shared/` is the mechanism built for this (mika#923): discovered by
# build.rs (`discover_support_dirs`) and seeded unconditionally, even under
# MIKA_DISABLE_BUNDLED_SKILLS.
#
# CONTRACT, inherited from dispatch-lib.sh: NO TOP-LEVEL IMPERATIVE CODE in this
# file — function definitions and constants only. That is what makes direct
# sourcing safe in the test harness, and what keeps the isolation audit moot.
#
# Test harness: skills/bundled/_shared/tests/test_pr_push_guard.sh
# Source scan:  scripts/check-pilot-push-sites.sh

# ── Protected branches. ──────────────────────────────────────────────────────
#
# Deliberate duplicate of GIT_OPS_PROTECTED_BRANCHES
# (crates/mika-agent/src/skills/builtin_handlers.rs). No cross-language single
# source exists here and inventing one would be disproportionate; the price is
# named rather than mechanised. If you change one, change the other.
PR_PUSH_GUARD_PROTECTED_BRANCHES="main master"

PR_PUSH_GUARD_TICKET="mika#2520"

# ── Paths the HANDLER itself materialises in the worktree. ───────────────────
#
# R5 refuses on a dirty tree, and a guard that fires on its own setup is a guard
# that gets disarmed. The handler copies the relay config into the worktree
# before spawning the pilot, and `.claude/claude-pilot.json` is a TRACKED file in
# this repo — so if the platform copy and the repo copy ever differ by a byte,
# an unqualified dirty-tree predicate would refuse every nominal dispatch.
#
# Deliberately a NAMED, NARROW list rather than excluding `.claude/` wholesale:
# the exemption covers what the handler writes, and nothing else the pilot might
# leave behind there.
PR_PUSH_GUARD_HANDLER_OWNED_PATHS=".claude/claude-pilot.json .claude/settings.local.json"

# ── Refusal reasons are a WIRE FORMAT. ───────────────────────────────────────
#
# They land in `tasks.result`, an operator reads them with `mika tasks get` and
# groups on them in SQL. Two spellings of one reason would split a population
# without saying so. One definition site; the test pins the list.
#
#   unresolved_branch  — the PR head could not be read (gh failed, empty field)
#   unresolved_base    — the PR base could not be read (fail-closed half of R3)
#   protected_branch   — the resolved target is main/master
#   branch_is_base     — the resolved target IS the PR's own base branch
#   head_mismatch      — the worktree's HEAD is detached, or on another branch
#   not_publishable    — rebase in progress, dirty tree, or no such remote branch
#   no_lease           — asked to push with no pre-session lease captured
#   no_pr_url          — no PR coordinates, so no head to resolve (see § 3.5)
#   repo_mismatch      — the worktree's origin is not the PR's repository
PR_PUSH_GUARD_REFUSAL_REASONS="unresolved_branch unresolved_base protected_branch branch_is_base head_mismatch not_publishable no_lease no_pr_url repo_mismatch"

# ── Resolved state, owned by this file. ──────────────────────────────────────
#
# The caller reads these; it never computes them. In particular it must never
# fall back to `git branch --show-current` (empty on a detached HEAD, `main`
# when HEAD is on main) or to the worktree's push configuration — AC4.
PR_PUSH_GUARD_BRANCH=""
PR_PUSH_GUARD_BASE=""
PR_PUSH_GUARD_REPO=""
PR_PUSH_GUARD_EXPECTED_SHA=""
PR_PUSH_GUARD_REFUSAL=""

# ── Compose a refusal. ───────────────────────────────────────────────────────
#
# The durable surface is `PR_PUSH_GUARD_REFUSAL`, which the caller routes into
# `RESULT` and therefore into `tasks.result`. stderr is written too, but is NOT
# sufficient: stderr produced before the pilot launches inherits the executor's
# `Stdio::piped()` handle, which the executor reads only in its
# `if !status.success()` branch — on a dispatch that succeeds those lines land in
# no file at all (the mika#2050 class, measured one file away).
#
# Every refusal names its LIFTING GESTURE. A refusal that does not name one is a
# refusal somebody works around by guesswork.
#
# PUBLIC on purpose: a caller that refuses for a reason it alone can see (no PR
# coordinates in its input, say) must compose the refusal HERE rather than
# hand-rolling the string, so the ticket prefix and the declared reason
# vocabulary above stay the single source of both.
#   Usage: pr_push_guard_refuse <reason> <detail> <remedy>
pr_push_guard_refuse() {
    local reason="$1" detail="$2" remedy="$3"
    PR_PUSH_GUARD_REFUSAL="REFUSED (${PR_PUSH_GUARD_TICKET}) — ${reason}: ${detail}. ${remedy}"
    echo "$PR_PUSH_GUARD_REFUSAL" >&2
    return 1
}

# Three entry points need a resolved branch before they can do anything, and
# each of them reaching that state means the caller skipped
# `pr_push_guard_resolve_target`. One refusal, one remedy.
#   Usage: _pr_push_guard_require_branch <what this blocks>
_pr_push_guard_require_branch() {
    [ -n "$PR_PUSH_GUARD_BRANCH" ] && return 0
    pr_push_guard_refuse "unresolved_branch" \
        "no target branch is resolved, so $1" \
        "Call pr_push_guard_resolve_target first; nothing downstream derives a branch of its own."
    return 1
}

# Reduce a git remote URL to `owner/repo`, whatever transport wrote it:
#   https://github.com/o/r.git   ssh://git@github.com/o/r   git@github.com:o/r
# Prints nothing and returns 1 when it cannot, so the caller fails closed.
_pr_push_guard_normalize_remote() {
    local url="${1%.git}"
    url="${url%/}"
    url="${url//:/\/}"            # scp-form and URL-form collapse to one shape
    local repo="${url##*/}"
    local rest="${url%/*}"
    local owner="${rest##*/}"
    [ -n "$owner" ] && [ -n "$repo" ] && [ "$owner" != "$repo" ] || return 1
    printf '%s/%s' "$owner" "$repo"
}

# ── R6 — the worktree's origin IS the PR's repository. ───────────────────────
#
# Without this term the push target's *branch* comes from the PR while the push
# target's *repository* comes from whatever `origin` the worktree happens to
# carry. A PR on another repository that resolves to a colliding worktree path
# would have its head branch pushed into THIS repository's origin. The branch
# name is attacker-choosable (anyone who can open a PR names its head), so the
# two halves must be checked against each other, not trusted separately.
#
# Fail-closed like the rest: an unreadable or unparseable origin refuses.
#
# Usage: _pr_push_guard_check_repo <worktree>
_pr_push_guard_check_repo() {
    local worktree="$1"

    [ -n "$PR_PUSH_GUARD_REPO" ] || return 0   # deprecated no-PR path: nothing to bind

    local origin_url normalized
    origin_url=$(git -C "$worktree" remote get-url origin 2>/dev/null) || origin_url=""

    if [ -z "$origin_url" ]; then
        pr_push_guard_refuse "repo_mismatch" \
            "the worktree '${worktree}' has no readable 'origin' remote, so it cannot be shown to be ${PR_PUSH_GUARD_REPO}" \
            "Nothing was pushed. Establish the worktree's remotes before re-dispatching."
        return 1
    fi

    normalized=$(_pr_push_guard_normalize_remote "$origin_url") || normalized=""

    if [ -z "$normalized" ]; then
        pr_push_guard_refuse "repo_mismatch" \
            "the worktree's origin '${origin_url}' does not reduce to an owner/repo pair, so it cannot be compared with ${PR_PUSH_GUARD_REPO}" \
            "Nothing was pushed. Establish the worktree's origin URL before re-dispatching."
        return 1
    fi

    # Forge owners and repositories are case-insensitive for identity purposes.
    if [ "$(printf '%s' "$normalized" | tr '[:upper:]' '[:lower:]')" \
       != "$(printf '%s' "$PR_PUSH_GUARD_REPO" | tr '[:upper:]' '[:lower:]')" ]; then
        pr_push_guard_refuse "repo_mismatch" \
            "the worktree's origin is '${normalized}' but the PR belongs to '${PR_PUSH_GUARD_REPO}'" \
            "Nothing was pushed. This is an upstream anomaly: establish why the dispatch paired this PR with this worktree before re-dispatching."
        return 1
    fi

    return 0
}

# ── R1 — resolve the target from the PR head, in ONE call. ───────────────────
#
# `headRefName` AND `baseRefName` come from the same `gh pr view`: the base is
# needed by R3 and a second round-trip would buy nothing. The handler already
# made this call to derive the worktree path and simply never let the value
# cross into the prompt — this is a propagation hole, not missing infrastructure.
#
# Usage: pr_push_guard_resolve_target <owner/repo> <pr-number>
pr_push_guard_resolve_target() {
    local repo_full="$1" pr_number="$2"

    PR_PUSH_GUARD_BRANCH=""
    PR_PUSH_GUARD_BASE=""
    PR_PUSH_GUARD_REPO=""
    PR_PUSH_GUARD_REFUSAL=""

    if [ -z "$repo_full" ] || [ -z "$pr_number" ]; then
        pr_push_guard_refuse "no_pr_url" \
            "no PR coordinates were supplied, so the push target cannot be derived from the PR head" \
            "Re-dispatch with pr_url. The deprecated worktree_path input resolves no branch, so the push is refused on that path by construction."
        return 1
    fi

    local json
    json=$(gh pr view "$pr_number" --repo "$repo_full" --json headRefName,baseRefName 2>/dev/null) || json=""

    local branch base
    branch=$(printf '%s' "$json" | jq -r '.headRefName // empty' 2>/dev/null) || branch=""
    base=$(printf '%s' "$json" | jq -r '.baseRefName // empty' 2>/dev/null) || base=""

    if [ -z "$branch" ]; then
        pr_push_guard_refuse "unresolved_branch" \
            "gh pr view returned no headRefName for ${repo_full}#${pr_number}" \
            "Check the PR exists and the gh token can read it, then re-dispatch. The target is never taken from the worktree's local push configuration."
        return 1
    fi

    # Fail-closed half of R3. An unreadable base means R3 cannot be evaluated,
    # and an unevaluated refusal term is not a satisfied one.
    if [ -z "$base" ]; then
        pr_push_guard_refuse "unresolved_base" \
            "gh pr view returned no baseRefName for ${repo_full}#${pr_number}, so the base-branch refusal cannot be evaluated" \
            "Check the PR exists and the gh token can read it, then re-dispatch."
        return 1
    fi

    PR_PUSH_GUARD_BRANCH="$branch"
    PR_PUSH_GUARD_BASE="$base"
    PR_PUSH_GUARD_REPO="$repo_full"
    return 0
}

# ── R2 + R3 — the target itself. ─────────────────────────────────────────────
#
# Reads only shell variables, so it is cheap and re-affirmable: it runs once
# before the pilot spawns (zero LLM cost on a refusal) and again immediately
# before the push, on variables the pilot cannot reach.
#
# R3 is the generalisation AC2 asks for in parentheses ("or any protected branch
# / base branch of a PR"): a PR stacked on `feat/x` has base `feat/x`, which is
# in no static list.
pr_push_guard_check_target() {
    PR_PUSH_GUARD_REFUSAL=""

    _pr_push_guard_require_branch "the target refusals cannot be evaluated" || return 1

    local branch="$PR_PUSH_GUARD_BRANCH" base="$PR_PUSH_GUARD_BASE"

    if [ -z "$base" ]; then
        pr_push_guard_refuse "unresolved_base" \
            "no base branch is resolved, so the base-branch refusal cannot be evaluated" \
            "Call pr_push_guard_resolve_target first."
        return 1
    fi

    local protected
    for protected in $PR_PUSH_GUARD_PROTECTED_BRANCHES; do
        if [ "$branch" = "$protected" ]; then
            pr_push_guard_refuse "protected_branch" \
                "the resolved push target is the protected branch '${branch}'" \
                "Nothing was pushed. This is an upstream anomaly, not a tuning question: establish why the PR's head is '${branch}' before re-dispatching."
            return 1
        fi
    done

    if [ "$branch" = "$base" ]; then
        pr_push_guard_refuse "branch_is_base" \
            "the resolved push target '${branch}' is the PR's own base branch" \
            "Nothing was pushed. Establish why head and base coincide on this PR before re-dispatching."
        return 1
    fi

    return 0
}

# ── Capture the lease BEFORE the session. ────────────────────────────────────
#
# A literal SHA read from the remote, not `origin/<branch>`: a pilot `git fetch`
# refreshes the tracking ref and would void a tracking-ref lease, which is the
# second axis of the founding defect (see the header).
#
# An empty `ls-remote` refuses instead of turning into a first push: an open PR
# implies its branch exists on the remote, so its absence means the resolution is
# looking at something else. Pushing there would create a branch on a false
# premise.
#
# Usage: pr_push_guard_capture_lease <worktree>
pr_push_guard_capture_lease() {
    local worktree="$1"

    PR_PUSH_GUARD_EXPECTED_SHA=""
    PR_PUSH_GUARD_REFUSAL=""

    _pr_push_guard_require_branch "no lease can be captured" || return 1

    # R6 before the network round-trip, and before the session: a worktree that
    # is not this PR's repository must cost zero LLM turns.
    _pr_push_guard_check_repo "$worktree" || return 1

    local sha
    sha=$(git -C "$worktree" ls-remote origin "refs/heads/${PR_PUSH_GUARD_BRANCH}" 2>/dev/null | head -n 1 | cut -f1) || sha=""

    if [ -z "$sha" ]; then
        pr_push_guard_refuse "not_publishable" \
            "origin has no refs/heads/${PR_PUSH_GUARD_BRANCH}, so there is no pre-session lease to take" \
            "Nothing was pushed. An open PR implies its head branch exists on the remote; establish why it does not before re-dispatching."
        return 1
    fi

    PR_PUSH_GUARD_EXPECTED_SHA="$sha"
    return 0
}

# ── R4 + R5 — the worktree's state after the session. ────────────────────────
#
# R4 closes the second reading of the rewind without having to settle which one
# actually happened: if HEAD moved onto `main`, we refuse rather than push main's
# tip onto the PR branch.
#
# Usage: pr_push_guard_check_worktree <worktree>
pr_push_guard_check_worktree() {
    local worktree="$1"

    PR_PUSH_GUARD_REFUSAL=""

    _pr_push_guard_require_branch "the worktree's HEAD cannot be checked against it" || return 1

    # R6 re-affirmed after the session: the pilot could have re-pointed origin.
    _pr_push_guard_check_repo "$worktree" || return 1

    local branch="$PR_PUSH_GUARD_BRANCH"

    # R4 — `symbolic-ref --short HEAD`, not `branch --show-current`: the latter
    # prints an empty line on a detached HEAD and exits 0, so its failure is
    # indistinguishable from an unnamed branch.
    local head
    head=$(git -C "$worktree" symbolic-ref --short HEAD 2>/dev/null) || head=""

    if [ -z "$head" ]; then
        pr_push_guard_refuse "head_mismatch" \
            "the worktree's HEAD is detached (or unreadable), so it cannot be shown to be on '${branch}'" \
            "Nothing was pushed. The rebase work is still in the worktree; check out '${branch}' there and inspect before re-dispatching."
        return 1
    fi

    if [ "$head" != "$branch" ]; then
        pr_push_guard_refuse "head_mismatch" \
            "the worktree's HEAD is on '${head}' but the PR head is '${branch}'" \
            "Nothing was pushed. The work is still in the worktree; establish why HEAD moved before re-dispatching."
        return 1
    fi

    # R5 — an in-flight rebase means the session stopped mid-resolution.
    local git_dir
    git_dir=$(git -C "$worktree" rev-parse --absolute-git-dir 2>/dev/null) || git_dir=""

    if [ -z "$git_dir" ]; then
        pr_push_guard_refuse "not_publishable" \
            "'${worktree}' does not read back as a git working tree" \
            "Nothing was pushed. Establish the worktree's state before re-dispatching."
        return 1
    fi

    if [ -d "${git_dir}/rebase-merge" ] || [ -d "${git_dir}/rebase-apply" ]; then
        pr_push_guard_refuse "not_publishable" \
            "a rebase is still in progress in the worktree" \
            "Nothing was pushed. The conflicts are unresolved and the work is intact; finish or abort the rebase in the worktree, then re-dispatch."
        return 1
    fi

    # Tracked changes only, minus the paths the handler materialises itself
    # (see PR_PUSH_GUARD_HANDLER_OWNED_PATHS). A positive `.` pathspec is
    # required alongside the exclusions — git refuses a pathspec list that is
    # exclusions only.
    local -a status_argv=(-C "$worktree" status --porcelain --untracked-files=no -- .)
    local owned
    for owned in $PR_PUSH_GUARD_HANDLER_OWNED_PATHS; do
        status_argv+=(":(exclude)${owned}")
    done

    local dirty dirty_rc=0
    dirty=$(git "${status_argv[@]}" 2>/dev/null) || dirty_rc=$?

    if [ "$dirty_rc" -ne 0 ]; then
        pr_push_guard_refuse "not_publishable" \
            "the worktree's status could not be read" \
            "Nothing was pushed. Establish the worktree's state before re-dispatching."
        return 1
    fi

    if [ -n "$dirty" ]; then
        pr_push_guard_refuse "not_publishable" \
            "the worktree has uncommitted tracked changes" \
            "Nothing was pushed. The changes are intact in the worktree; commit or discard them there, then re-dispatch."
        return 1
    fi

    # The remote branch must still be there. Re-checked after the session, not
    # only at lease time: it can be deleted while the pilot runs.
    local remote_sha
    remote_sha=$(git -C "$worktree" ls-remote origin "refs/heads/${branch}" 2>/dev/null | head -n 1 | cut -f1) || remote_sha=""

    if [ -z "$remote_sha" ]; then
        pr_push_guard_refuse "not_publishable" \
            "origin no longer has refs/heads/${branch}" \
            "Nothing was pushed. Establish why the PR's head branch disappeared before re-dispatching."
        return 1
    fi

    return 0
}

# ── THE SINGLE PUSH SITE. ────────────────────────────────────────────────────
#
# The argv is what AC1 prescribes, word for word:
#
#   git push --force-with-lease=<branch>:<expected-sha> origin HEAD:refs/heads/<branch>
#
#   * `--force-with-lease=<branch>:<sha>` — a literal SHA captured before the
#     session, so no fetch in between can void the lease.
#   * `origin HEAD:refs/heads/<branch>` — an explicit destination refspec, so
#     `push.default`, a stale local `main`, and a misconfigured upstream all stop
#     being able to decide where this lands.
#
# There is no retry, deliberately. The one legitimate failure of this push is
# "somebody pushed to the branch during the session"; re-leasing on the new SHA
# would overwrite their work — the same defect in a politer form. And a retry
# written into a PROMPT is precisely the improvisation surface where a bare
# `git push` gets typed: after a failure, under pressure. A transient network
# failure therefore costs one dispatch; the rebase commits stay in the worktree
# and the callback says so.
#
# Usage: pr_push_guard_push <worktree>
pr_push_guard_push() {
    local worktree="$1"

    # Worktree state after the session (R4, R5) ...
    pr_push_guard_check_worktree "$worktree" || return 1

    # ... then re-affirm the target on shell variables the pilot cannot touch.
    # Almost free, and it keeps the push site honest in front of a future edit
    # that mutates BRANCH in between.
    pr_push_guard_check_target || return 1

    if [ -z "$PR_PUSH_GUARD_EXPECTED_SHA" ]; then
        pr_push_guard_refuse "no_lease" \
            "no pre-session lease was captured, so this push would be unprotected" \
            "Nothing was pushed. Call pr_push_guard_capture_lease before the session; a lease taken after the pilot ran protects nothing."
        return 1
    fi

    git -C "$worktree" push \
        --force-with-lease="${PR_PUSH_GUARD_BRANCH}:${PR_PUSH_GUARD_EXPECTED_SHA}" \
        origin "HEAD:refs/heads/${PR_PUSH_GUARD_BRANCH}"
}
